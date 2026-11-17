//! Pure scheduler boundary for `glowtube.FaviconVideo`; the binary executes returned actions.
//! This avoids dt-t3 -> dt-render/media/platform dependencies and keeps encoding off API calls.

use dt_core::{Result, TrackInfo};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

/// Port `FaviconVideo`'s paths and measured show-latency setting.
#[derive(Clone, Debug)]
pub struct FaviconConfig {
    /// Dedicated clip directory; cleanup is limited to owned `glowtube-clip-*.svg` files here.
    pub clips_dir: PathBuf,
    /// Destination T3 project ID.
    pub project_id: String,
    /// Unique per-run component of every clip name, plus a monotonically increasing sequence.
    pub run_id: String,
    /// API-to-first-frame latency, default 500 ms (measured around 450 ms).
    pub show_latency: Duration,
}

/// Port `FaviconVideo._queue` request; binary adapts this to dt-render::FaviconEncodeRequest.
#[derive(Clone, Debug)]
pub struct ClipJob {
    /// Track identity for rejecting obsolete completions.
    pub track_id: String,
    /// Source media path.
    pub video: PathBuf,
    /// First frame's media time.
    pub start: Duration,
    /// Clip duration, including the scheduler's one-second overlap where applicable.
    pub duration: Duration,
    /// Unique absolute SVG path.
    pub output: PathBuf,
}

/// Port completed `FaviconVideo.pending` future result; feed it back on a later tick.
#[derive(Clone, Debug)]
pub struct ReadyClip {
    /// Original identity/start/output request.
    pub job: ClipJob,
    /// Actual frame count, determining end time at 24 fps.
    pub frames: u64,
}

/// Worker/API completions for `FaviconVideo.pending` and `_publish`, delivered by orchestration.
/// Identity/path lets the scheduler ignore stale results after a seek or track change.
#[derive(Clone, Debug)]
pub enum FaviconEvent {
    /// Encoding completed; actual frame count determines the clip's media end.
    Encoded(ReadyClip),
    /// Encoding failed/cancelled; clear this pending job and allow a retry.
    EncodeFailed(ClipJob),
    /// API accepted faviconPath; mark this path live only when it still matches the active job.
    Published(PathBuf),
    /// API failed; clear pending publish/live state so a later tick can retry or blank.
    PublishFailed(PathBuf),
}

/// Port `FaviconVideo._tick`'s snapshot of the player; media and monotonic wall time are separate.
pub struct FaviconTick<'a> {
    /// Current metadata, only Some when PCM/source is ready.
    pub track: Option<&'a TrackInfo>,
    /// Audible sample clock position.
    pub media_time: Duration,
    /// Wall clock for quiet-window deadlines.
    pub wall_time: Instant,
    /// Pause suppresses favicon and invalidates outstanding jobs.
    pub playing: bool,
    /// False for ctl favicon off.
    pub enabled: bool,
    /// Write loop acknowledgement: long write landed, publish allowed after its 450 ms settle.
    pub quiet_granted_at: Option<Instant>,
    /// Completed encode/publish operations since the last tick; discard stale/late results.
    pub events: Vec<FaviconEvent>,
}

/// Effects requested by `FaviconVideo._tick`, `_blank`, `_queue` and `_cleanup`.
#[derive(Clone, Debug)]
pub enum FaviconAction {
    /// Encode on a separate one-worker pool; never behind the six short-clip workers.
    Encode(ClipJob),
    /// Cancel pending encoding after seek, track change, pause or disable.
    CancelPending,
    /// Ask for a 2.5 s theme write and 2.2 s hold; do not publish until acknowledged/settled.
    RequestQuiet,
    /// Mutate the project's faviconPath through the API; return failure to the next tick owner.
    Publish(PathBuf),
    /// Publish the tiny blank SVG and expose theme fallback; do not delete recent clips.
    Blank,
    /// Cleanup owned clip files while retaining the newest 30 for late client fetches.
    Cleanup,
    /// Inform the write loop whether cheap hidden theme-video encoding is appropriate.
    SetLive(bool),
}

/// State machine for `glowtube.FaviconVideo`, without spawning threads or accessing user state.
pub struct FaviconScheduler {
    _private: (),
}

impl FaviconScheduler {
    /// Port `FaviconVideo.__init__` state, with injected paths. No HTTP/filesystem side effects.
    pub fn new(_config: FaviconConfig) -> Self {
        todo!("initialize favicon scheduler")
    }

    /// Port `FaviconVideo._tick`: tick about every 100 ms; first clip 8 s at now+4.5 s, grow
    /// 8 -> 24 -> 45 s plus overlap, reject >600 ms-late clips, and blank on seek/track changes.
    /// Publish only inside acknowledged quiet holds; aim >=96% visible and <=0.2 s A/V offset.
    pub fn tick(&mut self, _input: FaviconTick<'_>) -> Result<Vec<FaviconAction>> {
        todo!("port favicon scheduler tick")
    }

    /// Port `FaviconVideo.stop`: cancel pending work and request blank favicon before shutdown.
    pub fn stop(&mut self) -> Vec<FaviconAction> {
        todo!("stop favicon scheduler")
    }
}
