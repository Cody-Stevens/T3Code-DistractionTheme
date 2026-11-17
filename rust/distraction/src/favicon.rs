//! Execute favicon scheduler actions on independent encode/API workers.
use crate::{player::Player, worker::Worker, write_loop::WriteLoop};
use dt_core::{Error, Result, constants::FAVICON_KEEP};
use dt_t3::favicon::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Injectable scheduler boundary, including fake late quiet requests.
pub trait Scheduler {
    /// Advance from a snapshot and completed effects.
    fn tick(&mut self, input: FaviconTick<'_>) -> Result<Vec<FaviconAction>>;
    /// Request final cancellation/blanking.
    fn stop(&mut self) -> Vec<FaviconAction>;
}
impl Scheduler for FaviconScheduler {
    fn tick(&mut self, input: FaviconTick<'_>) -> Result<Vec<FaviconAction>> {
        self.tick(input)
    }
    fn stop(&mut self) -> Vec<FaviconAction> {
        self.stop()
    }
}
/// Blocking favicon effects; only encode/API worker threads call these during playback.
pub trait FaviconEffects: Send + Sync {
    /// Encode a clip on its independent worker.
    fn encode(&self, job: ClipJob) -> Result<ReadyClip>;
    /// Publish the selected deck's asset path.
    fn publish(&self, path: &Path) -> Result<()>;
    /// Clear both decks with a tiny owned blank SVG.
    fn blank(&self) -> Result<()>;
    /// Retain 30 recent owned clip files for late client downloads.
    fn cleanup(&self) -> Result<()>;
}
enum ApiJob {
    Publish(PathBuf, Instant),
    Blank,
    Cleanup,
}
type EncodeAnswer = (u64, Option<FaviconEvent>);
type ApiAnswer = (u64, Option<FaviconEvent>);
/// Joins independent favicon work and bridges quiet requests/permissions into the write loop.
pub struct FaviconDriver {
    scheduler: Box<dyn Scheduler>,
    effects: Arc<dyn FaviconEffects>,
    encode: Worker<(u64, ClipJob), EncodeAnswer>,
    api: Worker<(u64, ApiJob), ApiAnswer>,
    generation: Arc<AtomicU64>,
    front: Option<(char, Instant)>,
    deferred: Vec<FaviconEvent>,
    stopped: bool,
}
impl FaviconDriver {
    /// Construct with native or fake scheduler/effects; starts no media until asked by tick.
    pub fn new(scheduler: Box<dyn Scheduler>, effects: Arc<dyn FaviconEffects>) -> Result<Self> {
        let generation = Arc::new(AtomicU64::new(0));
        let enc_effects = effects.clone();
        let enc_gen = generation.clone();
        let encode = Worker::start(1, move |(generation, job): (u64, ClipJob)| {
            if generation != enc_gen.load(Ordering::Acquire) {
                return (generation, None);
            }
            let event = match enc_effects.encode(job.clone()) {
                Ok(clip) => FaviconEvent::Encoded(clip),
                Err(_) => FaviconEvent::EncodeFailed(job),
            };
            (generation, Some(event))
        })?;
        let api_effects = effects.clone();
        let api_gen = generation.clone();
        let api = Worker::start(8, move |(generation, job)| {
            if generation != api_gen.load(Ordering::Acquire) {
                return (generation, None);
            }
            let event = match job {
                ApiJob::Publish(path, deadline) => Some(
                    if Instant::now() < deadline && api_effects.publish(&path).is_ok() {
                        FaviconEvent::Published(path)
                    } else {
                        FaviconEvent::PublishFailed(path)
                    },
                ),
                ApiJob::Blank => {
                    let _ = api_effects.blank();
                    None
                }
                ApiJob::Cleanup => {
                    let _ = api_effects.cleanup();
                    None
                }
            };
            (generation, event)
        })?;
        Ok(Self {
            scheduler,
            effects,
            encode,
            api,
            generation,
            front: None,
            deferred: vec![],
            stopped: false,
        })
    }
    /// About 100 ms per tick; HTTP/encode work never blocks the write owner.
    pub fn tick(
        &mut self,
        player: &mut Player,
        writes: &mut WriteLoop,
        now: Instant,
    ) -> Result<()> {
        let mut events = std::mem::take(&mut self.deferred);
        let generation = self.generation.load(Ordering::Acquire);
        for (epoch, event) in self
            .encode
            .replies
            .try_iter()
            .chain(self.api.replies.try_iter())
        {
            if epoch == generation
                && let Some(event) = event
            {
                if let FaviconEvent::Published(path) = &event {
                    let name = path.file_name().and_then(|p| p.to_str()).unwrap_or("");
                    let deck = if name.starts_with("glowtube-clip-b-") {
                        'b'
                    } else {
                        'a'
                    };
                    self.front = Some((deck, now + Duration::from_millis(1200)));
                }
                events.push(event);
            }
        }
        if let Some((deck, _)) = self.front.filter(|(_, due)| now >= *due) {
            player.favicon_front = Some(deck);
            player.dirty = true;
            self.front = None;
        }
        let actions = self.scheduler.tick(FaviconTick {
            track: player.ready().map(|t| &t.info),
            media_time: player.clock().media_time(),
            wall_time: now,
            playing: player.playing,
            enabled: player.favicon_enabled,
            quiet_granted_at: writes.quiet_granted_at(),
            events,
        })?;
        for action in actions {
            let generation = self.generation.load(Ordering::Acquire);
            match action {
                FaviconAction::Encode(job) => {
                    if self.encode.submit((generation, job.clone())).is_err() {
                        self.deferred.push(FaviconEvent::EncodeFailed(job));
                    }
                }
                FaviconAction::CancelPending => {
                    self.generation.fetch_add(1, Ordering::AcqRel);
                    self.front = None;
                    self.deferred.clear();
                }
                FaviconAction::RequestQuiet => writes.request_quiet(),
                FaviconAction::Publish(path) => {
                    let granted = writes.quiet_granted_at().is_some_and(|grant| now >= grant)
                        && writes.timing.hold_until.is_some_and(|until| now < until)
                        && player.playing
                        && player.favicon_enabled;
                    if !granted
                        || self
                            .api
                            .submit((
                                generation,
                                ApiJob::Publish(
                                    path.clone(),
                                    writes.timing.hold_until.expect("granted hold"),
                                ),
                            ))
                            .is_err()
                    {
                        self.deferred.push(FaviconEvent::PublishFailed(path));
                    }
                }
                FaviconAction::Blank => self.api.submit((generation, ApiJob::Blank))?,
                FaviconAction::Cleanup => self.api.submit((generation, ApiJob::Cleanup))?,
                FaviconAction::SetLive(live) => {
                    if player.favicon_live != live {
                        player.dirty = true;
                    }
                    player.favicon_live = live;
                    if !live {
                        player.favicon_front = None;
                        self.front = None;
                    }
                }
            }
        }
        Ok(())
    }
    /// Wait out owned workers, then blank after any older in-flight publish has finished.
    pub fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        self.scheduler.stop();
        self.generation.fetch_add(1, Ordering::AcqRel);
        let encoding = self.encode.stop();
        let api = self.api.stop();
        let blank = self.effects.blank();
        encoding.and(api).and(blank)
    }
}
impl Drop for FaviconDriver {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Native adapters; a missing back deck is reported when asked to publish to it.
pub struct NativeEffects {
    /// Favicon AVIF backend, separate from short-clip workers.
    pub encoder: dt_render::encode::FfmpegAvifEncoder,
    /// Scoped authenticated T3 client (never logged).
    pub api: dt_t3::api::T3Api,
    /// Front deck project.
    pub project_a: String,
    /// Back deck project, if present.
    pub project_b: Option<String>,
    /// Dedicated local clip directory.
    pub clips_dir: PathBuf,
}
impl FaviconEffects for NativeEffects {
    fn encode(&self, job: ClipJob) -> Result<ReadyClip> {
        let encoded = dt_render::encode::encode_clip_svg(
            &self.encoder,
            &dt_render::encode::FaviconEncodeRequest {
                video: job.video.clone(),
                start: job.start,
                duration: job.duration,
                output: job.output.clone(),
            },
        )?;
        Ok(ReadyClip {
            job,
            frames: encoded.frames,
        })
    }
    fn publish(&self, path: &Path) -> Result<()> {
        let back = path
            .file_name()
            .and_then(|p| p.to_str())
            .is_some_and(|name| name.starts_with("glowtube-clip-b-"));
        let project = if back {
            self.project_b
                .as_deref()
                .ok_or(Error::Protocol("back deck not configured"))?
        } else {
            &self.project_a
        };
        self.api.update_project(
            project,
            [(
                "faviconPath".into(),
                serde_json::Value::String(path.to_string_lossy().into_owned()),
            )]
            .into(),
        )?;
        Ok(())
    }
    fn blank(&self) -> Result<()> {
        std::fs::create_dir_all(&self.clips_dir)?;
        let blank = self.clips_dir.join("glowtube-blank.svg");
        std::fs::write(
            &blank,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#,
        )?;
        for project in std::iter::once(&self.project_a).chain(self.project_b.iter()) {
            self.api.update_project(
                project,
                [(
                    "faviconPath".into(),
                    serde_json::Value::String(blank.to_string_lossy().into_owned()),
                )]
                .into(),
            )?;
        }
        Ok(())
    }
    fn cleanup(&self) -> Result<()> {
        if !self.clips_dir.exists() {
            return Ok(());
        }
        let mut files = vec![];
        for entry in std::fs::read_dir(&self.clips_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("glowtube-clip-")
                && name.ends_with(".svg")
                && entry.file_type()?.is_file()
            {
                files.push((entry.metadata()?.modified()?, entry.path()));
            }
        }
        files.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        for (_, path) in files.into_iter().skip(FAVICON_KEEP) {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
