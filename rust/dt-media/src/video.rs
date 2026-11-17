//! Bounded ffmpeg pipe reader, porting `glowtube.VideoReader`.

use dt_core::{Frame, Result};
use std::{path::PathBuf, sync::Arc};

/// Input and executable for a `VideoReader`; decode size and FPS come from dt-core.
#[derive(Clone, Debug)]
pub struct VideoReaderConfig {
    /// Source media file (local or downloaded).
    pub path: PathBuf,
    /// ffmpeg executable for RGB24 rawvideo output.
    pub ffmpeg: PathBuf,
}

/// Port `VideoReader.get/start_at/stop`. Calls must never wait for ffmpeg to produce a frame.
/// At 24 fps, keep about two seconds ahead/behind, return an older frame on a miss, and restart
/// decoding on backward seeks or requests more than two seconds beyond the decoded head.
pub trait VideoSource: Send {
    /// Return the requested/nearest older 640x360 RGB24 frame, or `None` while restarting/loading.
    fn get(&mut self, frame: u64) -> Result<Option<Arc<Frame>>>;
    /// Restart the pipe at a 24 fps frame index and invalidate the old buffer.
    fn start_at(&mut self, frame: u64) -> Result<()>;
    /// Stop/join the pipe worker and terminate its owned process; also clean up on drop.
    fn stop(&mut self) -> Result<()>;
}

/// Port `glowtube.VideoReader`: worker-owned ffmpeg stdout, aspect-fill scale/crop to 640x360.
pub struct VideoReader {
    _private: (),
}

impl VideoReader {
    /// Port `VideoReader.__init__`: start decoding frame zero into a bounded look-ahead buffer.
    pub fn open(_config: VideoReaderConfig) -> Result<Self> {
        todo!("open video pipe")
    }
}

impl VideoSource for VideoReader {
    fn get(&mut self, _frame: u64) -> Result<Option<Arc<Frame>>> {
        todo!("nonblocking frame lookup")
    }
    fn start_at(&mut self, _frame: u64) -> Result<()> {
        todo!("restart video pipe")
    }
    fn stop(&mut self) -> Result<()> {
        todo!("stop video pipe")
    }
}
