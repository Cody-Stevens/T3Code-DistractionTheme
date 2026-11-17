//! Media acquisition, decoding, analysis and speaker clock. All operations are phase-two stubs.
//! Ports `glowtube.expand`, `load`, `analyse`, `VideoReader` and the audio part of `Player`.
//! Calls that spawn tools run on workers, never in the audio callback or the 250 ms write loop.

pub mod audio;
pub mod download;
pub mod video;

pub use audio::{AudioOutput, AudioOutputConfig, CpalOutput, PcmAudio, analyse, decode_audio};
pub use download::{DownloadConfig, ExpandedTrack, LoadedTrack, expand, load};
pub use video::{VideoReader, VideoReaderConfig, VideoSource};
