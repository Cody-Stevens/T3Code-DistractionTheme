//! yt-dlp expansion and bounded cache loading, porting `glowtube.expand` and `load`.

use crate::PcmAudio;
use dt_core::{AudioFeatures, Result, TrackInfo};
use std::{path::PathBuf, sync::Arc};

/// Executables and cache location supplied by orchestration; no hidden working-directory state.
#[derive(Clone, Debug)]
pub struct DownloadConfig {
    /// yt-dlp executable; invoke with `--js-runtimes node` and no shell.
    pub yt_dlp: PathBuf,
    /// ffmpeg executable for 48 kHz stereo s16le audio decoding.
    pub ffmpeg: PathBuf,
    /// Download cache directory; retain media across PCM eviction.
    pub cache_dir: PathBuf,
}

/// One local file, single video, or nonempty playlist entry from `glowtube.expand`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedTrack {
    /// URL or local path to hand to `load`.
    pub url: String,
    /// Playlist/track title, possibly empty until `load` resolves it.
    pub title: String,
}

/// Ready result of `glowtube.load`; publish this atomically to the player after all work succeeds.
#[derive(Clone, Debug)]
pub struct LoadedTrack {
    /// Resolved ID, title, artist, local path and complete video-frame count.
    pub info: TrackInfo,
    /// Decoded audio shared with the output callback without copying.
    pub pcm: Arc<PcmAudio>,
    /// Analysis arrays with exactly `info.frames` entries, indexed at 24 fps.
    pub features: Arc<AudioFeatures>,
}

/// Port `glowtube.expand`: return ordered `(URL, title)` entries for a local file or yt-dlp link.
/// Use flat-playlist metadata, skip null entries, and preserve single-video URLs.
/// Blocking tool calls belong to the lazy-loader worker, outside the <100/200 ms theme budget.
pub fn expand(_url: &str, _config: &DownloadConfig) -> Result<Vec<ExpandedTrack>> {
    todo!("port glowtube.expand")
}

/// Port `glowtube.load`: reuse a valid cached path or download <=360p video, decode and analyse.
/// Return stereo i16 PCM at 48 kHz; truncate complete video frames at 24 fps.
/// The player retains PCM for only current/next tracks. Failure must not mark a partial load ready.
pub fn load(_track: &TrackInfo, _config: &DownloadConfig) -> Result<LoadedTrack> {
    todo!("port glowtube.load")
}
