//! Media acquisition, decoding, analysis and a latency-adjusted speaker clock.
//! Ports `glowtube.expand`, `load`, `analyse`, `VideoReader` and the audio part of `Player`.
//! Calls that spawn tools run on workers, never in the audio callback or the 250 ms write loop.

pub mod audio;
pub mod download;
mod output;
mod process;
pub mod video;

pub use audio::{AudioOutput, AudioOutputConfig, CpalOutput, PcmAudio, analyse, decode_audio};
pub use download::{DownloadConfig, ExpandedTrack, LoadedTrack, expand, load};
pub use video::{VideoReader, VideoReaderConfig, VideoSource};
