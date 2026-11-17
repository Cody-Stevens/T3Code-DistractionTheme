//! Bounded six-worker encode-ahead pool, keyed by source and six-frame grid.
use dt_core::{Error, Frame, Result, constants::*};
use dt_render::encode::{AvifEncoder, EncodedVideo};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};

/// Track identity plus exact source frame.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClipKey {
    /// Resolved source identity.
    pub track_id: String,
    /// Source start on the active grid.
    pub start_frame: u64,
}
/// Ready result, including the actual start (within one frame of the request).
#[derive(Clone, Debug)]
pub struct PreparedClip {
    /// Exact source position.
    pub key: ClipKey,
    /// Base64 and quality information, rechecked by the renderer.
    pub video: EncodedVideo,
}
/// Diagnostics matching Python's ahead counters.
#[derive(Clone, Debug, Default)]
pub struct AheadStats {
    /// Final renderer accepted an ahead result.
    pub used: u64,
    /// Completed result could not fit.
    pub too_big: u64,
    /// Matching work is pending.
    pub not_ready: u64,
    /// No matching job exists.
    pub none_queued: u64,
}
/// Injectable slow encoder; called only on pool workers.
pub trait AheadEncoder: Send + Sync {
    /// Fit a speed-6-class clip into the reserved base64 room.
    fn encode(&self, frames: &[Arc<Frame>], room: usize) -> Result<Option<EncodedVideo>>;
}
struct NativeEncoder(Arc<dyn AvifEncoder>);
impl AheadEncoder for NativeEncoder {
    fn encode(&self, frames: &[Arc<Frame>], room: usize) -> Result<Option<EncodedVideo>> {
        dt_render::encode::encode_to_fit(self.0.as_ref(), frames, room, 42)
    }
}
struct Job {
    generation: u64,
    key: ClipKey,
    frames: Vec<Arc<Frame>>,
    room: usize,
}
struct Answer {
    generation: u64,
    key: ClipKey,
    video: Option<EncodedVideo>,
}

/// At most twice the worker count of tracked/pending clips, with generation invalidation.
pub struct AheadPool {
    sender: Option<SyncSender<Job>>,
    replies: Receiver<Answer>,
    generation: Arc<AtomicU64>,
    jobs: HashMap<ClipKey, Option<EncodedVideo>>,
    workers: Vec<JoinHandle<()>>,
    capacity: usize,
    /// Counters for fallback decisions.
    pub stats: AheadStats,
}
impl AheadPool {
    /// Start native workers using the configured local encoder workspace.
    pub fn start(workers: usize) -> Result<Self> {
        Self::with_avif(workers, Arc::new(crate::runtime::encoder()?))
    }
    /// Adapt a shared AVIF backend to the size-targeting encoder.
    pub fn with_avif(workers: usize, encoder: Arc<dyn AvifEncoder>) -> Result<Self> {
        Self::with_encoder(workers, Arc::new(NativeEncoder(encoder)))
    }
    /// Start isolated workers with a fake encoder in tests.
    pub fn with_encoder(workers: usize, encoder: Arc<dyn AheadEncoder>) -> Result<Self> {
        if workers == 0 || workers > 64 {
            return Err(Error::InvalidInput("invalid ahead worker count"));
        }
        let capacity = workers * 2;
        let (sender, receive) = mpsc::sync_channel::<Job>(capacity);
        let receive = Arc::new(Mutex::new(receive));
        // Work admission bounds the number of answers; never lose a completion to a full reply queue.
        let (send, replies) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let mut pool = Self {
            sender: Some(sender),
            replies,
            generation,
            jobs: HashMap::new(),
            workers: vec![],
            capacity,
            stats: AheadStats::default(),
        };
        for _ in 0..workers {
            let receive = receive.clone();
            let send = send.clone();
            let generation = pool.generation.clone();
            let encoder = encoder.clone();
            pool.workers.push(
                thread::Builder::new()
                    .name("distraction-ahead".into())
                    .spawn(move || {
                        loop {
                            let job = match receive.lock() {
                                Ok(rx) => rx.recv(),
                                Err(_) => break,
                            };
                            let Ok(job) = job else {
                                break;
                            };
                            if job.generation != generation.load(Ordering::Acquire) {
                                continue;
                            }
                            let video = encoder.encode(&job.frames, job.room).ok().flatten();
                            if job.generation == generation.load(Ordering::Acquire) {
                                let _ = send.send(Answer {
                                    generation: job.generation,
                                    key: job.key,
                                    video,
                                });
                            }
                        }
                    })?,
            );
        }
        Ok(pool)
    }
    fn drain(&mut self) {
        while let Ok(answer) = self.replies.try_recv() {
            if answer.generation != self.generation.load(Ordering::Acquire) {
                continue;
            }
            if let Some(slot) = self.jobs.get_mut(&answer.key) {
                if let Some(video) = answer.video {
                    *slot = Some(video);
                } else {
                    self.jobs.remove(&answer.key);
                    self.stats.too_big += 1;
                }
            }
        }
    }
    /// Nonblocking submission; reserve 600 chars and reject duplicate or full work.
    pub fn submit(
        &mut self,
        key: ClipKey,
        frames: Vec<Arc<Frame>>,
        room_chars: usize,
    ) -> Result<bool> {
        self.drain();
        if frames.is_empty()
            || frames.len() > WRITE_SPAN_FRAMES as usize
            || frames
                .iter()
                .any(|f| f.width() != frames[0].width() || f.height() != frames[0].height())
        {
            return Err(Error::InvalidInput("invalid ahead frames"));
        }
        if room_chars <= AHEAD_SLACK_CHARS
            || self.jobs.contains_key(&key)
            || self.jobs.len() >= self.capacity
        {
            return Ok(false);
        }
        let Some(sender) = &self.sender else {
            return Ok(false);
        };
        let job = Job {
            generation: self.generation.load(Ordering::Acquire),
            key: key.clone(),
            frames,
            room: room_chars - AHEAD_SLACK_CHARS,
        };
        if sender.try_send(job).is_err() {
            return Ok(false);
        }
        self.jobs.insert(key, None);
        Ok(true)
    }
    /// Exact start preferred, then +/-1. Drop older/other-source results without waiting.
    pub fn take(&mut self, track_id: &str, start_frame: u64) -> Option<PreparedClip> {
        self.drain();
        self.jobs.retain(|key, _| {
            key.track_id == track_id && key.start_frame >= start_frame.saturating_sub(1)
        });
        for frame in [
            Some(start_frame),
            start_frame.checked_sub(1),
            start_frame.checked_add(1),
        ]
        .into_iter()
        .flatten()
        {
            let key = ClipKey {
                track_id: track_id.into(),
                start_frame: frame,
            };
            if self.jobs.get(&key).is_some_and(Option::is_some) {
                return Some(PreparedClip {
                    video: self.jobs.remove(&key).flatten().expect("ready entry"),
                    key,
                });
            }
        }
        if self
            .jobs
            .keys()
            .any(|key| key.track_id == track_id && key.start_frame.abs_diff(start_frame) <= 1)
        {
            self.stats.not_ready += 1;
        } else {
            self.stats.none_queued += 1;
        }
        None
    }
    /// Cancel queued work and reject late completions after seeks/pause/source changes.
    pub fn clear(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.jobs.clear();
        while self.replies.try_recv().is_ok() {}
    }
    /// Cancel pending work, disconnect the queue, and join all workers.
    pub fn stop(&mut self) -> Result<()> {
        self.clear();
        self.sender.take();
        let mut failed = false;
        for worker in self.workers.drain(..) {
            failed |= worker.join().is_err();
        }
        if failed {
            Err(Error::Protocol("ahead worker failed"))
        } else {
            Ok(())
        }
    }
}
impl Drop for AheadPool {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
