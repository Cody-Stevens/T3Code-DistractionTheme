//! Execute favicon scheduler actions on independent encode/API workers.
use crate::{player::Player, worker::Worker, write_loop::WriteLoop};
use dt_core::{Error, Result};
use dt_t3::favicon::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

/// Injectable scheduler boundary, including fake late quiet requests.
pub trait Scheduler {
    /// Advance from a snapshot and completed effects.
    fn tick(&mut self, input: FaviconTick<'_>, jumps: u64) -> Result<Vec<FaviconAction>>;
    /// Request final cancellation/blanking.
    fn stop(&mut self) -> Vec<FaviconAction>;
}
impl Scheduler for FaviconScheduler {
    fn tick(&mut self, input: FaviconTick<'_>, _jumps: u64) -> Result<Vec<FaviconAction>> {
        // The dt-t3 seek-counter follow-up adds this input to its public snapshot.
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
    fn publish(&self, project_id: &str, path: &Path) -> Result<()>;
    /// Clear the selected deck with a tiny owned blank SVG.
    fn blank(&self, project_id: &str, path: &Path) -> Result<()>;
    /// Retain the requested number of owned clip files for late client downloads.
    fn cleanup(&self, clips_dir: &Path, keep: usize) -> Result<()>;
}
enum ApiJob {
    Publish(String, PathBuf, Instant),
    Blank(String, PathBuf),
    Cleanup(PathBuf, usize),
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
                ApiJob::Publish(project_id, path, deadline) => Some(
                    if Instant::now() < deadline && api_effects.publish(&project_id, &path).is_ok()
                    {
                        FaviconEvent::Published(path)
                    } else {
                        FaviconEvent::PublishFailed(path)
                    },
                ),
                ApiJob::Blank(project_id, path) => {
                    let _ = api_effects.blank(&project_id, &path);
                    None
                }
                ApiJob::Cleanup(clips_dir, keep) => {
                    let _ = api_effects.cleanup(&clips_dir, keep);
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
                events.push(event);
            }
        }
        let actions = self.scheduler.tick(
            FaviconTick {
                track: player.ready().map(|t| &t.info),
                media_time: player.clock().media_time(),
                wall_time: now,
                playing: player.playing,
                enabled: player.favicon_enabled,
                quiet_granted_at: writes.quiet_written_at(),
                events,
            },
            player.jumps,
        )?;
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
                    self.deferred.clear();
                }
                FaviconAction::RequestQuiet => writes.request_quiet(),
                FaviconAction::Publish {
                    project_id, path, ..
                } => {
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
                                    project_id,
                                    path.clone(),
                                    writes.timing.hold_until.expect("granted hold"),
                                ),
                            ))
                            .is_err()
                    {
                        self.deferred.push(FaviconEvent::PublishFailed(path));
                    }
                }
                FaviconAction::Blank {
                    project_id, path, ..
                } => self
                    .api
                    .submit((generation, ApiJob::Blank(project_id, path)))?,
                FaviconAction::Cleanup { clips_dir, keep } => self
                    .api
                    .submit((generation, ApiJob::Cleanup(clips_dir, keep)))?,
                FaviconAction::SetFront(front) => {
                    let front = front.map(|deck| match deck {
                        Deck::A => 'a',
                        Deck::B => 'b',
                    });
                    if player.favicon_front != front {
                        player.favicon_front = front;
                        player.dirty = true;
                    }
                }
                FaviconAction::SetLive(live) => {
                    if player.favicon_live != live {
                        player.dirty = true;
                    }
                    player.favicon_live = live;
                    if !live {
                        player.favicon_front = None;
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
        let actions = self.scheduler.stop();
        self.generation.fetch_add(1, Ordering::AcqRel);
        let encoding = self.encode.stop();
        let api = self.api.stop();
        let mut blank = Ok(());
        for action in actions {
            if let FaviconAction::Blank {
                project_id, path, ..
            } = action
            {
                let result = self.effects.blank(&project_id, &path);
                if blank.is_ok() {
                    blank = result;
                }
            }
        }
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
    fn publish(&self, project_id: &str, path: &Path) -> Result<()> {
        self.api.update_project(
            project_id,
            [(
                "faviconPath".into(),
                serde_json::Value::String(path.to_string_lossy().into_owned()),
            )]
            .into(),
        )?;
        Ok(())
    }
    fn blank(&self, project_id: &str, path: &Path) -> Result<()> {
        if path.parent() != Some(self.clips_dir.as_path())
            || path
                .file_name()
                .is_none_or(|name| name != "glowtube-blank.svg")
        {
            return Err(Error::InvalidInput("blank path outside clip directory"));
        }
        std::fs::create_dir_all(&self.clips_dir)?;
        std::fs::write(path, BLANK_SVG)?;
        self.publish(project_id, path)
    }
    fn cleanup(&self, clips_dir: &Path, keep: usize) -> Result<()> {
        if clips_dir != self.clips_dir {
            return Err(Error::InvalidInput("cleanup outside clip directory"));
        }
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
        for (_, path) in files.into_iter().skip(keep) {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
