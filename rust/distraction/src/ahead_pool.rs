//! Six-worker, bounded encode-ahead pool from `glowtube.AheadEncoder`.

use dt_core::{Frame, Result};
use dt_render::encode::EncodedVideo;
use std::sync::Arc;

/// Port `AheadEncoder.jobs` keys; track identity prevents using clips from another track.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClipKey {
    /// Resolved track ID.
    pub track_id: String,
    /// First source frame on the six-frame audio grid.
    pub start_frame: u64,
}

/// Port successful `AheadEncoder.take` result, including the encoded clip's actual start.
#[derive(Clone, Debug)]
pub struct PreparedClip {
    /// Exact source start, permitted within +/-1 frame of requested start.
    pub key: ClipKey,
    /// Base64/width/quality, rechecked against final CSS room by the renderer.
    pub video: EncodedVideo,
}

/// Port `AheadEncoder.stats` counters for measurement tools and diagnostics.
#[derive(Clone, Debug, Default)]
pub struct AheadStats {
    /// Ahead clips accepted by the final CSS builder.
    pub used: u64,
    /// Completed encodes that cannot fit.
    pub too_big: u64,
    /// Matching job queued but not complete.
    pub not_ready: u64,
    /// No matching job queued.
    pub none_queued: u64,
}

/// Port `AheadEncoder`: keyed jobs, six workers, five writes/1.25 s ahead, bounded pending queue.
/// Reject duplicate keys and discard stale work on seek/track changes; favicon uses another pool.
pub struct AheadPool {
    _private: (),
}

impl AheadPool {
    /// Port `AheadEncoder.start`; default caller passes AHEAD_WORKERS=6, reject zero workers.
    pub fn start(_workers: usize) -> Result<Self> {
        todo!("start ahead workers")
    }

    /// Port `AheadEncoder.submit`: <=8 equal-sized source frames, last room minus 600 chars.
    /// Never block the write loop on encode or a full work queue; return false when not queued.
    pub fn submit(
        &mut self,
        _key: ClipKey,
        _frames: Vec<Arc<Frame>>,
        _room_chars: usize,
    ) -> Result<bool> {
        todo!("enqueue ahead clip")
    }

    /// Port `AheadEncoder.take`: nonblocking ready result within +/-1 frame, cancel older/other IDs.
    pub fn take(&mut self, _track_id: &str, _start_frame: u64) -> Option<PreparedClip> {
        todo!("take ready clip")
    }

    /// Port `AheadEncoder.clear`: cancel pending work and drop results on pause/seek/track changes.
    pub fn clear(&mut self) {
        todo!("invalidate ahead work")
    }

    /// Port `AheadEncoder.stop`: cancel, signal shutdown and join all owned encode workers/processes.
    pub fn stop(&mut self) -> Result<()> {
        todo!("stop ahead workers")
    }
}
