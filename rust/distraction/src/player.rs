//! Queue and current/next-only loading from `glowtube.Player`.
use crate::{control::ControlStatus, worker::Worker};
use dt_core::{Clock, Error, Result, TrackInfo, constants::*};
use dt_media::{AudioOutput, DownloadConfig, ExpandedTrack, LoadedTrack};
use dt_render::card::{CardContent, QueueItem, QueueState};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Residency of one queue entry; metadata survives eviction.
#[derive(Clone, Debug)]
pub enum LoadState {
    /// No resident audio.
    Unloaded,
    /// A loader owns the pending decode.
    Loading,
    /// Complete PCM and analysis, published together.
    Ready(Arc<LoadedTrack>),
    /// Failed loads are skipped during playback.
    Failed,
}
/// Queue metadata and residency state.
#[derive(Clone, Debug)]
pub struct QueueEntry {
    /// Resolved or provisional metadata.
    pub info: TrackInfo,
    /// Lazy loader state.
    pub state: LoadState,
}
/// Insertion order used by copied links and control commands.
#[derive(Clone, Copy, Debug)]
pub enum AddMode {
    /// Insert after current and play immediately.
    Now,
    /// Insert after current.
    Next,
    /// Append.
    End,
}
/// Blocking acquisition adapter; tests supply synthetic tracks.
pub trait MediaLoader: Send + Sync {
    /// Expand metadata without downloading all playlist entries.
    fn expand(&self, url: &str) -> Result<Vec<ExpandedTrack>>;
    /// Download/decode/analyse one entry completely.
    fn load(&self, track: &TrackInfo) -> Result<LoadedTrack>;
}
/// Adapter to the dt-media public interface.
pub struct NativeLoader(
    /// External tool paths and cache directory.
    pub DownloadConfig,
);
impl MediaLoader for NativeLoader {
    fn expand(&self, url: &str) -> Result<Vec<ExpandedTrack>> {
        dt_media::expand(url, &self.0)
    }
    fn load(&self, track: &TrackInfo) -> Result<LoadedTrack> {
        dt_media::load(track, &self.0)
    }
}
#[derive(Clone)]
struct SharedOutput(Arc<Mutex<Box<dyn AudioOutput>>>);
impl Clock for SharedOutput {
    fn media_time(&self) -> Duration {
        self.0
            .lock()
            .map(|audio| audio.media_time())
            .unwrap_or_default()
    }
}
type Expansion = (AddMode, Result<Vec<ExpandedTrack>>);
type Loading = (u64, Result<LoadedTrack>);

/// Single mutable owner of the queue and command/card state.
pub struct Player {
    /// Queue order, including previously played entries.
    pub queue: Vec<QueueEntry>,
    ids: Vec<u64>,
    serial: u64,
    /// Current queue index (zero for an empty queue).
    pub index: usize,
    /// Playback enabled, including while the source is loading.
    pub playing: bool,
    /// Main-loop lifetime flag.
    pub running: bool,
    /// Clamped gain.
    pub volume: f64,
    /// Clamped visual intensity.
    pub chaos: f64,
    /// Signed display delay adjustment.
    pub offset: f64,
    /// Last encoder quality label.
    pub quality: String,
    /// Whether favicon scheduling is enabled.
    pub favicon_enabled: bool,
    /// Whether a clip currently covers the theme video.
    pub favicon_live: bool,
    /// Front deck, supplied by the favicon adapter.
    pub favicon_front: Option<char>,
    /// Manual wall-time hold from ctl holdwrites.
    pub hold_until: Option<Instant>,
    /// Increments on seek/source/pause changes to invalidate rendering work.
    pub revision: u64,
    /// Commands request an early batch, subject to MIN_GAP and quiet holds.
    pub dirty: bool,
    message: String,
    message_until: Option<Instant>,
    audio: SharedOutput,
    source_id: Option<u64>,
    expanding: Worker<(String, AddMode), Expansion>,
    expansion_backlog: VecDeque<(String, AddMode)>,
    expansion_active: bool,
    loading: Worker<(u64, TrackInfo), Loading>,
    in_flight: Option<u64>,
}
impl Player {
    /// Open the native output and start independent metadata and lazy-load workers.
    pub fn new(urls: Vec<String>) -> Result<Self> {
        let loader = NativeLoader(crate::runtime::download_config()?);
        let audio = dt_media::CpalOutput::open(&dt_media::AudioOutputConfig::default())?;
        Self::with_dependencies(urls, Arc::new(loader), Box::new(audio))
    }
    /// Construct with fake media/output without touching devices, T3, or control ports.
    pub fn with_dependencies(
        urls: Vec<String>,
        loader: Arc<dyn MediaLoader>,
        mut audio: Box<dyn AudioOutput>,
    ) -> Result<Self> {
        let volume = std::env::var("GLOWTUBE_VOLUME")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(0.8)
            .clamp(0.0, 1.5);
        audio.set_volume(volume as f32)?;
        audio.set_playing(true)?;
        let expand = loader.clone();
        let expanding = Worker::start(1, move |(url, mode): (String, AddMode)| {
            (mode, expand.expand(&url))
        })?;
        let loading = Worker::start(1, move |(id, info)| (id, loader.load(&info)))?;
        let mut player = Self {
            queue: vec![],
            ids: vec![],
            serial: 0,
            index: 0,
            playing: true,
            running: true,
            volume,
            chaos: 11.0,
            offset: 0.16,
            quality: "starting".into(),
            favicon_enabled: true,
            favicon_live: false,
            favicon_front: None,
            hold_until: None,
            revision: 0,
            dirty: true,
            message: String::new(),
            message_until: None,
            audio: SharedOutput(Arc::new(Mutex::new(audio))),
            source_id: None,
            expanding,
            expansion_backlog: VecDeque::new(),
            expansion_active: false,
            loading,
            in_flight: None,
        };
        for url in urls {
            player.add(url, AddMode::End)?;
        }
        Ok(player)
    }
    /// Clone the audible clock handle; it never executes loading/rendering work.
    pub fn clock(&self) -> Arc<dyn Clock> {
        Arc::new(self.audio.clone())
    }
    fn output<T>(&self, apply: impl FnOnce(&mut dyn AudioOutput) -> Result<T>) -> Result<T> {
        let mut audio = self
            .audio
            .0
            .lock()
            .map_err(|_| Error::Protocol("audio control failed"))?;
        apply(audio.as_mut())
    }
    /// Current entry, if any.
    pub fn track(&self) -> Option<&QueueEntry> {
        self.queue.get(self.index)
    }
    /// Current ready source; callers clone only for the duration of a batch.
    pub fn ready(&self) -> Option<&Arc<LoadedTrack>> {
        match &self.track()?.state {
            LoadState::Ready(track) => Some(track),
            _ => None,
        }
    }
    /// Queue URL expansion without running external tools on the owner thread.
    pub fn add(&mut self, url: String, mode: AddMode) -> Result<()> {
        self.expansion_backlog.push_back((url, mode));
        self.expand_next()?;
        self.say("Fetching the link…".into(), Duration::from_secs(60));
        Ok(())
    }
    fn expand_next(&mut self) -> Result<()> {
        if !self.expansion_active
            && let Some(request) = self.expansion_backlog.pop_front()
        {
            self.expanding.submit(request)?;
            self.expansion_active = true;
        }
        Ok(())
    }
    fn insert(&mut self, tracks: Vec<ExpandedTrack>, mode: AddMode) -> Result<()> {
        let count = tracks.len();
        let what = if count == 1 {
            tracks[0].title.clone()
        } else if count == 0 {
            "nothing".into()
        } else {
            format!("{count} tracks")
        };
        let at = if self.queue.is_empty() {
            0
        } else {
            match mode {
                AddMode::End => self.queue.len(),
                _ => self.index + 1,
            }
        };
        let entries: Vec<_> = tracks
            .into_iter()
            .map(|track| {
                let mut info = TrackInfo::new(track.url);
                if !track.title.is_empty() {
                    info.title = track.title;
                }
                QueueEntry {
                    info,
                    state: LoadState::Unloaded,
                }
            })
            .collect();
        let ids: Vec<_> = (0..count)
            .map(|_| {
                self.serial += 1;
                self.serial
            })
            .collect();
        let was_empty = self.queue.is_empty();
        self.queue.splice(at..at, entries);
        self.ids.splice(at..at, ids);
        if count > 0 && !was_empty && matches!(mode, AddMode::Now) {
            self.index = at;
            self.set_playing(true)?;
            self.source_changed()?;
        }
        self.say(
            format!(
                "{}{what}",
                match mode {
                    AddMode::Now => "Playing ",
                    AddMode::Next => "Up next: ",
                    AddMode::End => "Queued: ",
                }
            ),
            Duration::from_secs(4),
        );
        Ok(())
    }
    /// Publish completions, evict old PCM, start at most one lazy load and advance EOF.
    pub fn poll(&mut self) -> Result<()> {
        while let Ok((mode, result)) = self.expanding.replies.try_recv() {
            self.expansion_active = false;
            match result {
                Ok(tracks) => self.insert(tracks, mode)?,
                Err(_) => self.say("Could not open that link".into(), Duration::from_secs(4)),
            }
        }
        self.expand_next()?;
        while let Ok((id, result)) = self.loading.replies.try_recv() {
            self.in_flight = None;
            if let Some(index) = self.ids.iter().position(|&entry| entry == id) {
                let wanted = index == self.index || index == self.index + 1;
                self.queue[index].state = match result {
                    Ok(track) => {
                        self.queue[index].info = track.info.clone();
                        if wanted {
                            LoadState::Ready(Arc::new(track))
                        } else {
                            LoadState::Unloaded
                        }
                    }
                    Err(_) => LoadState::Failed,
                };
                self.dirty = true;
            }
        }
        let failed = self
            .track()
            .is_some_and(|t| matches!(t.state, LoadState::Failed));
        let ended = self.source_id.is_some() && self.output(|a| Ok(a.ended()))?;
        if (self.playing && ended) || (failed && self.queue.len() > 1) {
            self.jump(1)?;
        }
        self.evict();
        let wanted_source = self
            .ids
            .get(self.index)
            .copied()
            .filter(|_| self.ready().is_some());
        if wanted_source != self.source_id {
            let pcm = self.ready().map(|track| track.pcm.clone());
            self.output(|audio| audio.set_source(pcm))?;
            self.source_id = wanted_source;
            self.revision += 1;
            self.dirty = true;
        }
        if self.in_flight.is_none() {
            for index in self.index..=(self.index + 1).min(self.queue.len().saturating_sub(1)) {
                if let Some(entry) = self.queue.get_mut(index)
                    && matches!(entry.state, LoadState::Unloaded)
                {
                    let id = self.ids[index];
                    self.loading.submit((id, entry.info.clone()))?;
                    entry.state = LoadState::Loading;
                    self.in_flight = Some(id);
                    self.dirty = true;
                    break;
                }
            }
        }
        Ok(())
    }
    fn evict(&mut self) {
        for (index, entry) in self.queue.iter_mut().enumerate() {
            if index != self.index
                && index != self.index + 1
                && matches!(entry.state, LoadState::Ready(_))
            {
                entry.state = LoadState::Unloaded;
            }
        }
    }
    fn source_changed(&mut self) -> Result<()> {
        self.output(|audio| audio.set_source(None))?;
        self.source_id = None;
        self.evict();
        self.invalidate();
        Ok(())
    }
    /// Invalidate source-dependent rendering and favicon work.
    pub fn invalidate(&mut self) {
        self.revision += 1;
        self.dirty = true;
    }
    /// Wrap a signed queue delta and reset audio position.
    pub fn jump(&mut self, delta: i32) -> Result<()> {
        if !self.queue.is_empty() {
            self.index =
                (self.index as i64 + i64::from(delta)).rem_euclid(self.queue.len() as i64) as usize;
            self.source_changed()?;
        }
        Ok(())
    }
    /// Move a zero-based upcoming offset to next (offset zero is already next).
    pub fn play_next(&mut self, offset: usize) -> Result<()> {
        if let Some(src) = (self.index + 1)
            .checked_add(offset)
            .filter(|&src| offset > 0 && src < self.queue.len())
        {
            let entry = self.queue.remove(src);
            let id = self.ids.remove(src);
            self.say(
                format!("Up next: {}", entry.info.title),
                Duration::from_secs(4),
            );
            self.queue.insert(self.index + 1, entry);
            self.ids.insert(self.index + 1, id);
            self.evict();
        }
        Ok(())
    }
    /// Freeze/resume audio, invalidating the frame grid on changes.
    pub fn set_playing(&mut self, playing: bool) -> Result<()> {
        self.output(|audio| audio.set_playing(playing))?;
        if self.playing != playing {
            self.invalidate();
        }
        self.playing = playing;
        Ok(())
    }
    /// Set clamped finite gain.
    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        if !volume.is_finite() {
            return Err(Error::InvalidInput("volume must be finite"));
        }
        let volume = volume.clamp(0.0, 1.5);
        self.output(|audio| audio.set_volume(volume as f32))?;
        self.volume = volume;
        Ok(())
    }
    fn seek_absolute(&mut self, seconds: f64) -> Result<()> {
        if let Some(track) = self.ready() {
            let last = track.pcm.samples.len().saturating_sub(1) as f64 / f64::from(SAMPLE_RATE);
            self.output(|audio| audio.seek(Duration::from_secs_f64(seconds.clamp(0.0, last))))?;
            self.invalidate();
        }
        Ok(())
    }
    /// Relative seek clamped to the last PCM sample.
    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        if !seconds.is_finite() {
            return Err(Error::InvalidInput("seek must be finite"));
        }
        self.seek_absolute(self.audio.media_time().as_secs_f64() + seconds)
    }
    /// Fractional seek clamped to 0..=1.
    pub fn seek_to(&mut self, fraction: f64) -> Result<()> {
        if !fraction.is_finite() {
            return Err(Error::InvalidInput("seek must be finite"));
        }
        if let Some(track) = self.ready() {
            self.seek_absolute(
                fraction.clamp(0.0, 1.0) * track.pcm.samples.len().saturating_sub(1) as f64
                    / f64::from(SAMPLE_RATE),
            )?;
        }
        Ok(())
    }
    /// Show a transient card subtitle; expired text remains in the wire status as in Python.
    pub fn say(&mut self, message: String, duration: Duration) {
        self.say_at(message, duration, Instant::now());
    }
    /// Deterministic variant for card deadline tests.
    pub fn say_at(&mut self, message: String, duration: Duration, now: Instant) {
        self.message = message;
        self.message_until = now.checked_add(duration);
        self.dirty = true;
    }
    /// Cache key for the renderer, including only four visible upcoming rows.
    pub fn card_content(&self, subtitle: String, now: Instant) -> CardContent {
        CardContent {
            title: self
                .track()
                .map(|t| t.info.title.clone())
                .unwrap_or_else(|| "Nothing playing".into()),
            subtitle: if self.message_until.is_some_and(|until| now < until) {
                self.message.clone()
            } else {
                subtitle
            },
            playing: self.playing,
            upcoming: self
                .queue
                .iter()
                .skip(self.index + 1)
                .take(CARD_QUEUE_ROWS)
                .map(|entry| QueueItem {
                    title: entry.info.title.clone(),
                    state: match entry.state {
                        LoadState::Loading => QueueState::Loading,
                        LoadState::Failed => QueueState::Failed,
                        _ => QueueState::Normal,
                    },
                })
                .collect(),
        }
    }
    /// Read-only Python-compatible snapshot.
    pub fn status(&self) -> ControlStatus {
        ControlStatus {
            track: self.track().map(|t| t.info.title.clone()),
            playing: self.playing,
            status: self.message.clone(),
            time: (self.audio.media_time().as_secs_f64() * 10.0).round_ties_even() / 10.0,
            queue: self.queue.iter().map(|t| t.info.title.clone()).collect(),
            index: self.index,
            volume: self.volume,
            chaos: self.chaos,
            offset: (self.offset * 1000.0).round_ties_even() / 1000.0,
            quality: self.quality.clone(),
        }
    }
    /// Stop output and join workers; attempt all cleanup even after one failure.
    pub fn stop(&mut self) -> Result<()> {
        self.running = false;
        self.expansion_backlog.clear();
        let audio = self.output(|audio| audio.stop());
        let expanding = self.expanding.stop();
        let loading = self.loading.stop();
        while self.loading.replies.try_recv().is_ok() {}
        self.queue.iter_mut().for_each(|entry| {
            if matches!(entry.state, LoadState::Ready(_)) {
                entry.state = LoadState::Unloaded;
            }
        });
        audio.and(expanding).and(loading)
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
/// Run native adapters and the loopback server until ctl quit or Ctrl-C.
pub fn play(urls: Vec<String>) -> Result<()> {
    crate::runtime::play(urls)
}
