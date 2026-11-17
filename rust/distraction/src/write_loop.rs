//! Audio-clock-grid batching and the acknowledged favicon quiet-window handshake.
use crate::{
    ahead_pool::{AheadPool, ClipKey},
    player::Player,
};
use dt_core::{AudioFeatureFrame, Clock, Error, Frame, Result, TrackInfo, constants::*};
use dt_media::{VideoReader, VideoReaderConfig, VideoSource};
use dt_render::{
    card::{Card, CardContent, CardRenderOptions},
    css::{BuiltTheme, CssValues, DynamicInput, DynamicState},
    encode::AvifEncoder,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Separate monotonic clock for debounce/quiet holds; audio alone controls frame scheduling.
pub trait WallClock: Send + Sync {
    /// Current monotonic time.
    fn now(&self) -> Instant;
}
/// Production wall clock.
pub struct SystemWallClock;
impl WallClock for SystemWallClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Python's build estimate, signed display offset, next_k and quiet hold.
#[derive(Clone, Debug)]
pub struct WriteTiming {
    /// Smoothed build cost (.7 old + .3 measured).
    pub build_estimate: Duration,
    /// Signed write-to-display offset in seconds.
    pub display_offset: f64,
    /// Next frame on the six-frame grid anchored at the last batch.
    pub next_frame: Option<u64>,
    /// Monotonic hold deadline.
    pub hold_until: Option<Instant>,
}
impl Default for WriteTiming {
    fn default() -> Self {
        Self {
            build_estimate: Duration::from_millis(150),
            display_offset: 0.16,
            next_frame: None,
            hold_until: None,
        }
    }
}
/// Rendering boundary; the native implementation delegates codec/ladder decisions to dt-render.
pub trait Renderer {
    /// Open a nonblocking source for the ready track.
    fn video(&mut self, track: &TrackInfo) -> Result<Box<dyn VideoSource>>;
    /// Draw one precomputed glow.
    fn glow(&mut self, frame: &Frame, features: AudioFeatureFrame) -> Result<Frame>;
    /// Edge palette used in sparse keyframes.
    fn colors(&mut self, frame: &Frame) -> Result<[String; 5]>;
    /// Render/cache card state and fit the complete dynamic theme.
    fn build(&mut self, content: CardContent, input: RenderBatch<'_>) -> Result<BuiltTheme>;
    /// Complete loading/idle card theme.
    fn idle(&mut self, content: CardContent, extra_css: &str) -> Result<BuiltTheme>;
    /// Last quality description for status JSON.
    fn quality(&self) -> String;
}
/// Borrowed per-write input, independent of the renderer's card cache.
pub struct RenderBatch<'a> {
    /// Aligned source frames.
    pub frames: &'a [Arc<Frame>],
    /// Aligned precomputed glow frames.
    pub glows: &'a [Arc<Frame>],
    /// Sparse animation keyframes.
    pub values: &'a [CssValues],
    /// Current playback state.
    pub playing: bool,
    /// Visual intensity.
    pub chaos: f32,
    /// Matching ahead clip, checked against final room by the renderer.
    pub prepared: Option<&'a dt_render::encode::EncodedVideo>,
    /// Cheap mode for hidden video or a long quiet batch.
    pub cheap: bool,
    /// Front-deck/measurement rules, included before fitting the video.
    pub extra_css: &'a str,
}
/// Native renderer with a single-owner dynamic state and equality-keyed card cache.
pub struct NativeRenderer {
    /// Shared backend; ahead workers use independent state through the same encoder.
    pub encoder: Arc<dyn AvifEncoder>,
    /// Font directory.
    pub card_options: CardRenderOptions,
    /// Video decoder executable.
    pub ffmpeg: std::path::PathBuf,
    state: DynamicState,
    card: Option<(CardContent, Card)>,
    measure: bool,
}
impl NativeRenderer {
    /// Construct without opening video or drawing anything.
    pub fn new(
        encoder: Arc<dyn AvifEncoder>,
        card_options: CardRenderOptions,
        ffmpeg: std::path::PathBuf,
    ) -> Self {
        Self {
            encoder,
            card_options,
            ffmpeg,
            state: DynamicState::default(),
            card: None,
            measure: std::env::var_os("GLOWTUBE_MEASURE").is_some_and(|value| !value.is_empty()),
        }
    }
    fn card(&mut self, content: CardContent) -> Result<&Card> {
        if self.card.as_ref().is_none_or(|(key, _)| key != &content) {
            let card = dt_render::card::render(&content, &self.card_options)?;
            self.card = Some((content, card));
        }
        Ok(&self.card.as_ref().expect("rendered card").1)
    }
}
impl Renderer for NativeRenderer {
    fn video(&mut self, track: &TrackInfo) -> Result<Box<dyn VideoSource>> {
        Ok(Box::new(VideoReader::open(VideoReaderConfig {
            path: track
                .path
                .clone()
                .ok_or(Error::InvalidInput("ready track has no video path"))?,
            ffmpeg: self.ffmpeg.clone(),
        })?))
    }
    fn glow(&mut self, frame: &Frame, features: AudioFeatureFrame) -> Result<Frame> {
        dt_render::glow_frame(frame, features)
    }
    fn colors(&mut self, frame: &Frame) -> Result<[String; 5]> {
        dt_render::edge_colors(frame)
    }
    fn build(&mut self, content: CardContent, input: RenderBatch<'_>) -> Result<BuiltTheme> {
        let card = self.card(content)?.clone();
        let mut extra_css = input.extra_css.to_owned();
        if self.measure {
            extra_css.push_str(
                r#"[data-workspace-area="conversation"]::after{background:#ff00ff!important}"#,
            );
        }
        dt_render::css::build_dynamic(
            self.encoder.as_ref(),
            &mut self.state,
            DynamicInput {
                frames: input.frames,
                glows: input.glows,
                values: input.values,
                card: &card,
                playing: input.playing,
                chaos: input.chaos,
                prepared: input.prepared,
                cheap: input.cheap,
                extra_css: &extra_css,
            },
        )
    }
    fn idle(&mut self, content: CardContent, extra_css: &str) -> Result<BuiltTheme> {
        let mut css = dt_render::css::idle_theme(self.card(content)?)?;
        css.push_str(extra_css);
        Ok(BuiltTheme {
            css,
            video_room: 30000,
        })
    }
    fn quality(&self) -> String {
        if self.state.quality.is_empty() {
            "starting".into()
        } else {
            self.state.quality.clone()
        }
    }
}
/// Atomic theme write adapter, injectable for timing/failure tests.
pub trait ThemeSink {
    /// Persist the complete standalone stylesheet.
    fn write(&mut self, theme: &BuiltTheme) -> Result<()>;
}
impl ThemeSink for dt_t3::theme::ThemeWriter {
    fn write(&mut self, theme: &BuiltTheme) -> Result<()> {
        dt_t3::theme::ThemeWriter::write(
            self,
            &dt_t3::theme::ThemeDocument::glowtube(theme.css.clone())?,
        )
    }
}
/// Owns video, card/render state, ahead workers, and write acknowledgement state.
pub struct WriteLoop {
    clock: Arc<dyn Clock>,
    wall: Arc<dyn WallClock>,
    /// Scheduling state, exposed for diagnostics.
    pub timing: WriteTiming,
    renderer: Box<dyn Renderer>,
    sink: Box<dyn ThemeSink>,
    ahead: AheadPool,
    reader: Option<Box<dyn VideoSource>>,
    revision: Option<u64>,
    playing: bool,
    last_write: Option<Instant>,
    last_css: Option<String>,
    next_idle: Option<Instant>,
    quiet_requested: bool,
    batch_quiet: bool,
    quiet_granted: Option<Instant>,
    room: usize,
}
impl WriteLoop {
    /// Construct native adapters; no theme is written until tick/run.
    pub fn new(clock: Arc<dyn Clock>, timing: WriteTiming) -> Result<Self> {
        let encoder = Arc::new(crate::runtime::encoder()?);
        let renderer = NativeRenderer::new(
            encoder.clone(),
            crate::runtime::card_options(),
            "ffmpeg".into(),
        );
        let paths = dt_t3::T3Paths::discover()?;
        Self::with_dependencies(
            clock,
            Arc::new(SystemWallClock),
            timing,
            Box::new(renderer),
            Box::new(dt_t3::theme::ThemeWriter {
                themes_dir: paths.themes_dir,
            }),
            AheadPool::with_avif(AHEAD_WORKERS, encoder)?,
        )
    }
    /// Fake clocks/renderer/sink keep tests entirely independent of live devices/T3.
    pub fn with_dependencies(
        clock: Arc<dyn Clock>,
        wall: Arc<dyn WallClock>,
        timing: WriteTiming,
        renderer: Box<dyn Renderer>,
        sink: Box<dyn ThemeSink>,
        ahead: AheadPool,
    ) -> Result<Self> {
        if !timing.display_offset.is_finite() {
            return Err(Error::InvalidInput("nonfinite display offset"));
        }
        Ok(Self {
            clock,
            wall,
            timing,
            renderer,
            sink,
            ahead,
            reader: None,
            revision: None,
            playing: true,
            last_write: None,
            last_css: None,
            next_idle: None,
            quiet_requested: false,
            batch_quiet: false,
            quiet_granted: None,
            room: 30000,
        })
    }
    /// A scheduler request remains pending until a successful long write lands.
    pub fn request_quiet(&mut self) {
        self.quiet_requested = true;
    }
    /// Grant time is in the future for 450 ms after the long write; scheduler must check it.
    pub fn quiet_granted_at(&self) -> Option<Instant> {
        self.quiet_granted
    }
    /// Clear source-dependent results and quiet permissions on seek/pause/source changes.
    pub fn invalidate(&mut self) -> Result<()> {
        self.timing.next_frame = None;
        self.ahead.clear();
        self.batch_quiet = false;
        self.quiet_requested = false;
        self.quiet_granted = None;
        if let Some(mut reader) = self.reader.take() {
            reader.stop()?;
        }
        Ok(())
    }
    fn sync(&mut self, player: &Player) -> Result<()> {
        if self.revision != Some(player.revision) {
            // Preserve a request made before the very first batch.
            if self.revision.is_some() {
                self.invalidate()?;
            }
            self.revision = Some(player.revision);
        }
        self.playing = player.playing;
        self.timing.display_offset = player.offset;
        Ok(())
    }
    /// Synchronize discontinuities before ticking the favicon scheduler.
    pub fn synchronize(&mut self, player: &Player) -> Result<()> {
        self.sync(player)
    }
    /// Build eight frames, one paused frame, or exactly 60 frames for a pending quiet request.
    pub fn batch(&mut self, player: &mut Player) -> Result<Option<BuiltTheme>> {
        self.batch_quiet = false;
        self.sync(player)?;
        let Some(track) = player.ready().cloned().filter(|t| t.info.frames > 0) else {
            return Ok(None);
        };
        if self.reader.is_none() {
            self.reader = Some(self.renderer.video(&track.info)?);
        }
        let is_quiet = player.playing && self.quiet_requested;
        let count = if is_quiet {
            (QUIET_CONTENT.as_secs_f64() * f64::from(FPS)).round() as u64
        } else if player.playing {
            u64::from(WRITE_SPAN_FRAMES)
        } else {
            1
        };
        let time = self.clock.media_time().as_secs_f64();
        let start = if player.playing {
            time + self.timing.build_estimate.as_secs_f64() + self.timing.display_offset
        } else {
            time
        };
        let mut k0 = (start * f64::from(FPS)).round_ties_even().max(0.0) as u64;
        k0 = k0.min(track.info.frames - 1);
        if player.playing
            && let Some(next) = self
                .timing
                .next_frame
                .filter(|next| next.abs_diff(k0) <= 2 && *next < track.info.frames)
        {
            k0 = next;
        }
        let cheap = player.favicon_live || is_quiet;
        let prepared = if player.playing && !cheap {
            self.ahead.take(&track.info.id, k0)
        } else {
            None
        };
        if let Some(prepared) = &prepared {
            k0 = prepared.key.start_frame;
        }
        let end = (k0 + count).min(track.info.frames);
        let mut frames = vec![];
        let mut glows = vec![];
        let mut values = vec![];
        let mut last = None;
        for k in k0..end {
            let frame = self
                .reader
                .as_mut()
                .expect("video source")
                .get(k)?
                .or_else(|| last.clone());
            let Some(frame) = frame else {
                continue;
            };
            last = Some(frame.clone());
            let feature = track
                .features
                .get(k as usize)
                .ok_or(Error::Protocol("analysis shorter than track"))?;
            glows.push(Arc::new(self.renderer.glow(&frame, feature)?));
            let mut value = CssValues::new();
            value.insert("bs".into(), format!("{:.2}", feature.bass));
            value.insert("kk".into(), format!("{:.2}", feature.kick));
            let i = k - k0;
            let final_frame = k == end - 1;
            if i.is_multiple_of(6) || final_frame {
                value.insert(
                    "pr".into(),
                    format!(
                        "{:.4}",
                        k as f64 / track.info.frames.saturating_sub(1).max(1) as f64
                    ),
                );
                let colors = self.renderer.colors(&frame)?;
                for (j, color) in colors.into_iter().enumerate() {
                    value.insert(if j == 4 { "aa".into() } else { format!("c{j}") }, color);
                }
            }
            if i.is_multiple_of(2) || final_frame {
                for (band, value_band) in feature.bands.iter().enumerate() {
                    value.insert(format!("b{band}"), format!("{value_band:.2}"));
                }
            }
            frames.push(frame);
            values.push(value);
        }
        if frames.is_empty() {
            return Ok(None);
        }
        // A short tail or missing initial video frames cannot promise 2.5 seconds of coverage.
        let long_coverage = is_quiet && frames.len() == count as usize;
        let elapsed = k0 / u64::from(FPS);
        let total = track.info.frames / u64::from(FPS);
        let clock = format!(
            "{}:{:02} / {}:{:02}",
            elapsed / 60,
            elapsed % 60,
            total / 60,
            total % 60
        );
        let subtitle = if track.info.artist.is_empty() {
            clock
        } else {
            format!("{}  ·  {clock}", track.info.artist)
        };
        let content = player.card_content(subtitle, self.wall.now());
        let extra = front_css(player.favicon_front);
        let built = self.renderer.build(
            content,
            RenderBatch {
                frames: &frames,
                glows: &glows,
                values: &values,
                playing: player.playing,
                chaos: player.chaos as f32,
                prepared: prepared.as_ref().map(|p| &p.video),
                cheap,
                extra_css: &extra,
            },
        )?;
        self.room = built.video_room;
        player.quality = self.renderer.quality();
        if prepared.is_some() {
            if player.quality.contains("ahead") {
                self.ahead.stats.used += 1;
            } else {
                self.ahead.stats.too_big += 1;
            }
        }
        if player.playing && !is_quiet {
            self.timing.next_frame = Some(k0 + u64::from(WRITE_STEP_FRAMES));
            if !cheap {
                for n in 1..=AHEAD_WRITES {
                    let start = k0 + n as u64 * u64::from(WRITE_STEP_FRAMES);
                    if start >= track.info.frames {
                        break;
                    }
                    let mut raw = vec![];
                    for frame in
                        start..(start + u64::from(WRITE_SPAN_FRAMES)).min(track.info.frames)
                    {
                        if let Some(frame) =
                            self.reader.as_mut().expect("video source").get(frame)?
                        {
                            raw.push(frame);
                        } else {
                            raw.clear();
                            break;
                        }
                    }
                    if !raw.is_empty() {
                        self.ahead.submit(
                            ClipKey {
                                track_id: track.info.id.clone(),
                                start_frame: start,
                            },
                            raw,
                            self.room,
                        )?;
                    }
                }
            }
        } else {
            self.timing.next_frame = None;
            self.ahead.clear();
        }
        self.batch_quiet = long_coverage;
        Ok(Some(built))
    }
    /// Audio-grid delay, clamped to 0..500 ms; never derive it from wall uptime.
    pub fn until_next(&self) -> Duration {
        let Some(next) = self.timing.next_frame.filter(|_| self.playing) else {
            return WRITE_INTERVAL;
        };
        let ideal = self.clock.media_time().as_secs_f64()
            + self.timing.build_estimate.as_secs_f64()
            + self.timing.display_offset;
        Duration::from_secs_f64((next as f64 / f64::from(FPS) - ideal).clamp(0.0, 0.5))
    }
    /// Delay until work can run, combining audio scheduling with monotonic safety holds.
    pub fn delay(&self, player: &Player) -> Duration {
        let now = self.wall.now();
        let mut delay = if player.dirty {
            Duration::ZERO
        } else if let Some(idle) = self.next_idle {
            idle.saturating_duration_since(now)
        } else if self.quiet_requested && player.playing {
            Duration::ZERO
        } else {
            self.until_next()
        };
        for deadline in [
            self.last_write.and_then(|t| t.checked_add(MIN_WRITE_GAP)),
            self.timing.hold_until,
            player.hold_until,
        ]
        .into_iter()
        .flatten()
        {
            delay = delay.max(deadline.saturating_duration_since(now));
        }
        delay
    }
    /// One bounded owner iteration; acknowledge ONLY a successful long batch write.
    pub fn tick(&mut self, player: &mut Player) -> Result<bool> {
        self.sync(player)?;
        if !self.delay(player).is_zero() {
            return Ok(false);
        }
        let started = self.wall.now();
        let result = self.batch(player);
        let long_batch = result.as_ref().is_ok_and(|theme| theme.is_some()) && self.batch_quiet;
        let built = match result {
            Ok(Some(built)) => {
                self.next_idle = if !player.playing || self.timing.next_frame.is_none() {
                    Some(self.wall.now() + WRITE_INTERVAL)
                } else {
                    None
                };
                built
            }
            Ok(None) | Err(_) => {
                self.batch_quiet = false;
                self.timing.next_frame = None;
                self.next_idle = Some(self.wall.now() + WRITE_INTERVAL);
                let subtitle = if player.track().is_some() {
                    "Loading…"
                } else {
                    "Copy a YouTube link, then press Play"
                };
                self.renderer.idle(
                    player.card_content(subtitle.into(), self.wall.now()),
                    &front_css(player.favicon_front),
                )?
            }
        };
        let measured = self
            .wall
            .now()
            .saturating_duration_since(started)
            .as_secs_f64();
        self.timing.build_estimate = Duration::from_secs_f64(
            0.7 * self.timing.build_estimate.as_secs_f64() + 0.3 * measured,
        );
        if built.css.chars().count() > CSS_LIMIT {
            self.batch_quiet = false;
            return Err(Error::InvalidInput("theme exceeds CSS limit"));
        }
        player.dirty = false;
        if self.last_css.as_ref() == Some(&built.css) {
            return Ok(false);
        }
        if let Err(error) = self.sink.write(&built) {
            player.dirty = true;
            self.batch_quiet = false;
            self.timing.next_frame = None;
            return Err(error);
        }
        let written = self.wall.now();
        self.last_write = Some(written);
        self.last_css = Some(built.css);
        if long_batch && self.quiet_requested {
            self.quiet_requested = false;
            self.batch_quiet = false;
            self.timing.hold_until = Some(written + QUIET_HOLD);
            self.quiet_granted = Some(written + QUIET_SETTLE);
        }
        Ok(true)
    }
    /// Simple embedding loop; native runtime adds HTTP/input/favicon dispatch around tick.
    pub fn run(&mut self, player: &mut Player) -> Result<()> {
        while player.running {
            player.poll()?;
            if let Err(error) = self.tick(player) {
                eprintln!("batch failed: {error}");
            }
            std::thread::sleep(
                self.delay(player)
                    .clamp(Duration::from_millis(1), Duration::from_millis(50)),
            );
        }
        Ok(())
    }
    /// Stop video and ahead workers, attempting both even on failure.
    pub fn stop(&mut self) -> Result<()> {
        let video = if let Some(mut reader) = self.reader.take() {
            reader.stop()
        } else {
            Ok(())
        };
        let ahead = self.ahead.stop();
        video.and(ahead)
    }
    /// Final standalone idle theme after resources stop; preserve debounce and active quiet hold.
    pub fn finish_idle(&mut self) -> Result<()> {
        let now = self.wall.now();
        let deadline = [
            self.last_write.map(|t| t + MIN_WRITE_GAP),
            self.timing.hold_until,
        ]
        .into_iter()
        .flatten()
        .max();
        if let Some(deadline) = deadline {
            std::thread::sleep(deadline.saturating_duration_since(now));
        }
        let theme = self.renderer.idle(
            CardContent {
                title: "Nothing playing".into(),
                subtitle: "Copy a YouTube link, then press Play".into(),
                playing: false,
                upcoming: vec![],
            },
            "",
        )?;
        if theme.css.chars().count() > CSS_LIMIT {
            return Err(Error::InvalidInput("theme exceeds CSS limit"));
        }
        self.sink.write(&theme)
    }
}
impl Drop for WriteLoop {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
/// Measured Python selector; carried even by idle writes while a deck is selected.
pub fn front_css(deck: Option<char>) -> String {
    match deck {
        Some(deck @ ('a' | 'b')) => {
            format!("img[src*=\"glowtube-clip-{deck}-\"]{{z-index:2147483001!important}}")
        }
        _ => String::new(),
    }
}
