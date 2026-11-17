//! PCM, FFT envelopes and latency-adjusted output, from `glowtube.analyse` and `Player`.

use dt_core::{AudioFeatures, Clock, Result};
use std::{path::Path, sync::Arc, time::Duration};

/// Stereo sample frames (`[left, right]`) at `dt_core::constants::SAMPLE_RATE`.
/// Port of `Track.pcm`; duration is `samples.len() / 48000`, not scalar-sample count.
#[derive(Clone, Debug)]
pub struct PcmAudio {
    /// Signed 16-bit samples, ready for the callback; empty sources must analyse to empty arrays.
    pub samples: Vec<[i16; 2]>,
}

/// Port `glowtube.load`'s ffmpeg command: decode an input file to 48 kHz, stereo, s16le PCM.
/// Reject partial sample frames and nonzero process exits. Work is blocking and off the write loop.
pub fn decode_audio(_path: &Path, _ffmpeg: &Path) -> Result<PcmAudio> {
    todo!("port load's PCM decode")
}

/// Port `glowtube.analyse`: 4096-sample Hann FFT, 16 log bands (40..14000 Hz), bass 28..150 Hz.
/// Normalize band percentiles 8/98.5 with release .8; bass 30/99.5, power 1.4, release .72.
/// Kick: positive flux, +/-45-frame threshold, >6-frame spacing, .82 decay; ring 1440 px/s.
/// Return one entry per complete 24 fps frame, including safe empty/short/silent input handling.
pub fn analyse(_pcm: &PcmAudio) -> Result<AudioFeatures> {
    todo!("port glowtube.analyse")
}

/// Device selection for `Player.__init__`'s low-latency output stream.
#[derive(Clone, Debug, Default)]
pub struct AudioOutputConfig {
    /// `None` selects the default output; unsupported 48 kHz stereo formats return an error.
    pub device_name: Option<String>,
}

/// Port `Player._callback`, `seek`, `seek_to` and `media_time`, isolated from queue ownership.
/// The callback emits silence when paused/unloaded and zero-fills the final partial block.
/// Implementations must not allocate, spawn tools, or hold a player lock in the device callback.
pub trait AudioOutput: Clock {
    /// Replace the source and reset its position; keep its `Arc` alive while callbacks read it.
    fn set_source(&mut self, pcm: Option<Arc<PcmAudio>>) -> Result<()>;
    /// Freeze/unfreeze audible media time and sample advancement.
    fn set_playing(&mut self, playing: bool) -> Result<()>;
    /// Seek to an absolute sample position, clamped to source length; invalidate prior clock state.
    fn seek(&mut self, position: Duration) -> Result<()>;
    /// Set gain in 0..=1.5, matching the `vol` control command.
    fn set_volume(&mut self, volume: f32) -> Result<()>;
    /// Signal end-of-source to orchestration; queue transitions remain the player's job.
    fn ended(&self) -> bool;
    /// Stop the stream and join owned callback resources before returning.
    fn stop(&mut self) -> Result<()>;
}

/// Windows-first cpal/WASAPI implementation of `Player._callback` and `media_time`.
/// Own device-thread resources here; publish clock snapshots through thread-safe state.
pub struct CpalOutput {
    _private: (),
}

impl CpalOutput {
    /// Port `Player.__init__`: open a low-latency 48 kHz stereo output without starting media.
    pub fn open(_config: &AudioOutputConfig) -> Result<Self> {
        todo!("open cpal output")
    }
}

impl Clock for CpalOutput {
    fn media_time(&self) -> Duration {
        todo!("latency-adjusted audible sample clock")
    }
}

impl AudioOutput for CpalOutput {
    fn set_source(&mut self, _pcm: Option<Arc<PcmAudio>>) -> Result<()> {
        todo!("replace PCM")
    }
    fn set_playing(&mut self, _playing: bool) -> Result<()> {
        todo!("pause output")
    }
    fn seek(&mut self, _position: Duration) -> Result<()> {
        todo!("seek PCM")
    }
    fn set_volume(&mut self, _volume: f32) -> Result<()> {
        todo!("set gain")
    }
    fn ended(&self) -> bool {
        todo!("callback EOF notification")
    }
    fn stop(&mut self) -> Result<()> {
        todo!("stop output")
    }
}
