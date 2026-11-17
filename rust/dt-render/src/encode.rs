//! Animated AVIF contracts from `encoder.py` and `glowtube.avif/encode_video`.
//! An AV1 bitstream alone or a still AVIF item does not satisfy this API: Chromium must animate it.

use dt_core::{Frame, Result};
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Quality/speed intent from Pillow's libavif settings; ffmpeg CPU-used needs measured mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeSpeed {
    /// Ahead clip: match speed-6-class quality (reference takes 200..400 ms for eight frames).
    Ahead,
    /// Long favicon: match reference speed 8 and quality 60.
    Favicon,
    /// Immediate fallback or glow: match reference speed 10, within the write budget.
    Fast,
}

/// Port `encoder._encode` / `glowtube.avif` keyword arguments.
#[derive(Clone, Debug)]
pub struct AvifOptions {
    /// Pillow-style quality intent (0..=100); do not copy directly into libaom CRF.
    pub quality: u8,
    /// Reference quality/speed class, to calibrate at equal size.
    pub speed: EncodeSpeed,
    /// Positive duration for each input frame in milliseconds, same length as the frame slice.
    /// Short clips use round(1000/24)=42; favicon clips use cumulative 41/42 ms rounding.
    pub durations_ms: Vec<u32>,
    /// Total plays: reference `loop=1` means a single play, then hold the final frame.
    pub plays: u32,
    /// Encoder thread cap: 2 ahead, 4 immediate, 6 favicon in the reference.
    pub threads: usize,
}

/// Backend for `encoder._encode`: equal-sized, nonempty RGB24 frames to an animated AVIF file.
/// Reject mismatched durations/dimensions. Output must animate all frames in Chromium 152.
/// Implementations own and reap child processes and keep scratch files inside the given work dir.
pub trait AvifEncoder: Send + Sync {
    /// Encode a sequence including its AVIF animation container, timing and loop count.
    fn encode(&self, frames: &[Arc<Frame>], options: &AvifOptions) -> Result<Vec<u8>>;
}

/// Planned external backend for `encoder._encode`, using libaom-av1 and ffmpeg's AVIF muxer.
/// For sequences, disable still-picture mode; mux to a seekable file rather than stdout.
#[derive(Clone, Debug)]
pub struct FfmpegAvifEncoder {
    /// ffmpeg executable; startup validation must check libaom-av1 and the AVIF muxer.
    pub ffmpeg: PathBuf,
    /// Private temporary workspace for seekable AVIF output; remove only owned temporary files.
    pub work_dir: PathBuf,
}

impl AvifEncoder for FfmpegAvifEncoder {
    fn encode(&self, _frames: &[Arc<Frame>], _options: &AvifOptions) -> Result<Vec<u8>> {
        todo!("animated ffmpeg AVIF backend; measure Chromium animation before enabling")
    }
}

/// Port `encoder.encode_to_fit`'s successful `(base64, width, quality)` result.
#[derive(Clone, Debug)]
pub struct EncodedVideo {
    /// AVIF bytes encoded as ASCII base64, without a data URL prefix.
    pub base64: String,
    /// Encoded width; height is the corresponding rounded 16:9 value.
    pub width: u32,
    /// Quality intent actually selected.
    pub quality: u8,
}

/// Port `encoder.encode_to_fit`: try widths 640/512/384; two fast q20/q45 probes estimate size,
/// then target 85% of base64 room with speed-6-class encode and one retry at quality minus six.
/// Return `None` if nothing fits; caller reserves 600 chars and rechecks the final CSS budget.
pub fn encode_to_fit(
    _encoder: &dyn AvifEncoder,
    _frames: &[Arc<Frame>],
    _room_chars: usize,
    _frame_ms: u32,
) -> Result<Option<EncodedVideo>> {
    todo!("port encoder.encode_to_fit")
}

/// Port `glowtube.encode_video`: immediate speed-10-class fallback at a reference LADDER index.
/// Use 42 ms frames at 24 fps; reject invalid levels. Caller rechecks base64 against CSS room.
pub fn encode_video(
    _encoder: &dyn AvifEncoder,
    _frames: &[Arc<Frame>],
    _level: usize,
) -> Result<EncodedVideo> {
    todo!("port glowtube.encode_video")
}

/// Port `glowtube.avif`: encode RGB frames at the requested quality with fast speed intent,
/// 42 ms/frame, four threads and a single play, returning base64 without the data URL prefix.
/// Used for glow clips at q30; nonempty, equal-sized input is required.
pub fn avif(_encoder: &dyn AvifEncoder, _frames: &[Arc<Frame>], _quality: u8) -> Result<String> {
    todo!("port glowtube.avif")
}

/// Port `encoder.encode_clip_svg` input. Decode with ffmpeg at 640x360 / 24 fps by default.
#[derive(Clone, Debug)]
pub struct FaviconEncodeRequest {
    /// Local video to decode, never a command-line string for a shell.
    pub video: PathBuf,
    /// First media position in the source.
    pub start: Duration,
    /// Requested clip length, with scheduler-provided overlap.
    pub duration: Duration,
    /// Unique absolute `glowtube-clip-<run>-<sequence>.svg` destination.
    pub output: PathBuf,
}

/// Port `encoder.encode_clip_svg`'s `(path, count)` return.
#[derive(Clone, Debug)]
pub struct EncodedFavicon {
    /// Published SVG file, padded to at least 4,300,000 ASCII bytes (>4 MiB).
    pub path: PathBuf,
    /// Actual complete frames, used to derive end media time without duration drift.
    pub frames: u64,
}

/// Port `encoder.encode_clip_svg`: ffmpeg decode, animated AVIF, base64 `<image>` inside SVG.
/// Default q60, favicon speed, six threads, single play. Durations use cumulative rounded
/// milliseconds (41/42) totaling exactly 1000 ms per 24 frames. Reject empty decode, pad past
/// 4 MiB, and return the actual count. Dedicated favicon worker must not queue behind short clips.
pub fn encode_clip_svg(
    _encoder: &FfmpegAvifEncoder,
    _request: &FaviconEncodeRequest,
) -> Result<EncodedFavicon> {
    todo!("port encoder.encode_clip_svg")
}
