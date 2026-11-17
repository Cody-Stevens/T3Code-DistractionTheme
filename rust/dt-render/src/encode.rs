//! Animated AVIF contracts from `encoder.py` and `glowtube.avif/encode_video`.
//! An AV1 bitstream alone or a still AVIF item does not satisfy this API: Chromium must animate it.

use dt_core::{Frame, Result};
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Quality/speed intent from Pillow's libavif settings; ffmpeg CPU-used needs measured mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

/// Experimental external backend: ffmpeg libaom IVF packets plus the in-process AVIF muxer.
/// NativeAvifEncoder is the measured production choice; this backend trails its matched-size quality.
#[derive(Clone, Debug)]
pub struct FfmpegAvifEncoder {
    /// ffmpeg executable; startup validation must check libaom-av1 and the AVIF muxer.
    pub ffmpeg: PathBuf,
    /// Retained configuration field; the streaming IVF backend creates no scratch files.
    pub work_dir: PathBuf,
}

impl AvifEncoder for FfmpegAvifEncoder {
    fn encode(&self, frames: &[Arc<Frame>], options: &AvifOptions) -> Result<Vec<u8>> {
        validate(frames, options)?;
        let mut worker = Worker::start(&self.ffmpeg, frames, options)?;
        worker.encode(frames, options)
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
    encoder: &dyn AvifEncoder,
    frames: &[Arc<Frame>],
    room_chars: usize,
    frame_ms: u32,
) -> Result<Option<EncodedVideo>> {
    if frames.is_empty() || frame_ms == 0 {
        return Err(dt_core::Error::InvalidInput("empty encode-ahead input"));
    }
    if room_chars == 0 {
        return Ok(None);
    }
    let target = 0.85 * room_chars as f64;
    for width in [640, 512, 384] {
        let picked = resize_all(frames, width)?;
        let probe = |quality, speed| -> Result<String> {
            let opts = options(picked.len(), quality, speed, frame_ms, 2);
            Ok(b64(&encoder.encode(&picked, &opts)?))
        };
        let lo = probe(20, EncodeSpeed::Fast)?.len();
        let hi = probe(45, EncodeSpeed::Fast)?.len();
        if lo > room_chars && width != 384 {
            continue;
        }
        let slope = (hi as f64).ln() - (lo as f64).ln();
        let slope = if slope == 0.0 { 0.03 } else { slope / 25.0 };
        let quality = (20.0 + (target.ln() - (lo as f64).ln()) / slope).clamp(10.0, 70.0) as u8;
        for q in [quality, quality.saturating_sub(6).max(10)] {
            let data = probe(q, EncodeSpeed::Ahead)?;
            if data.len() <= room_chars {
                return Ok(Some(EncodedVideo {
                    base64: data,
                    width,
                    quality: q,
                }));
            }
        }
    }
    Ok(None)
}

/// Port `glowtube.encode_video`: immediate speed-10-class fallback at a reference LADDER index.
/// Use 42 ms frames at 24 fps; reject invalid levels. Caller rechecks base64 against CSS room.
pub fn encode_video(
    encoder: &dyn AvifEncoder,
    frames: &[Arc<Frame>],
    level: usize,
) -> Result<EncodedVideo> {
    let &(width, quality) = LADDER
        .get(level)
        .ok_or(dt_core::Error::InvalidInput("invalid video ladder level"))?;
    let picked = resize_all(frames, width)?;
    Ok(EncodedVideo {
        base64: avif(encoder, &picked, quality)?,
        width,
        quality,
    })
}

/// Port `glowtube.avif`: encode RGB frames at the requested quality with fast speed intent,
/// 42 ms/frame, four threads and a single play, returning base64 without the data URL prefix.
/// Used for glow clips at q30; nonempty, equal-sized input is required.
pub fn avif(encoder: &dyn AvifEncoder, frames: &[Arc<Frame>], quality: u8) -> Result<String> {
    Ok(b64(&encoder.encode(
        frames,
        &options(frames.len(), quality, EncodeSpeed::Fast, 42, 4),
    )?))
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
    encoder: &FfmpegAvifEncoder,
    request: &FaviconEncodeRequest,
) -> Result<EncodedFavicon> {
    encode_clip_svg_with(&NativeAvifEncoder, &encoder.ffmpeg, request, false)
}
#[path = "avif_mux.rs"]
mod avif_mux;
use dt_core::Error;
use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Mutex, mpsc},
    thread::{self, JoinHandle},
};

/// The Python fallback ladder; all entries preserve 24 fps.
pub const LADDER: [(u32, u8); 12] = [
    (640, 60),
    (640, 52),
    (640, 45),
    (640, 38),
    (640, 32),
    (640, 26),
    (640, 21),
    (512, 24),
    (512, 18),
    (384, 20),
    (384, 14),
    (320, 10),
];
fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}
fn options(n: usize, quality: u8, speed: EncodeSpeed, ms: u32, threads: usize) -> AvifOptions {
    AvifOptions {
        quality,
        speed,
        durations_ms: vec![ms; n],
        plays: 1,
        threads,
    }
}
fn resize_all(frames: &[Arc<Frame>], width: u32) -> Result<Vec<Arc<Frame>>> {
    if frames.is_empty() {
        return Err(Error::InvalidInput("empty AVIF sequence"));
    }
    frames
        .iter()
        .map(|f| crate::resized(f, width, width * 9 / 16).map(Arc::new))
        .collect()
}
fn validate(frames: &[Arc<Frame>], o: &AvifOptions) -> Result<()> {
    let Some(first) = frames.first() else {
        return Err(Error::InvalidInput("empty AVIF sequence"));
    };
    if o.quality > 100
        || o.threads == 0
        || o.threads > 64
        || o.durations_ms.len() != frames.len()
        || o.durations_ms.contains(&0)
        || first.width() > 65535
        || first.height() > 65535
        || frames
            .iter()
            .any(|f| f.width() != first.width() || f.height() != first.height())
    {
        return Err(Error::InvalidInput(
            "invalid AVIF options or frame dimensions",
        ));
    }
    Ok(())
}
fn command(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
}
fn terminate(child: &mut Child) {
    #[cfg(windows)]
    {
        let _ = command(Path::new("taskkill"))
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkerKey {
    width: u32,
    height: u32,
    count: usize,
    quality: u8,
    speed: EncodeSpeed,
    threads: usize,
}
impl WorkerKey {
    fn new(frames: &[Arc<Frame>], o: &AvifOptions) -> Self {
        Self {
            width: frames[0].width(),
            height: frames[0].height(),
            count: frames.len(),
            quality: o.quality,
            speed: o.speed,
            threads: o.threads,
        }
    }
}
struct Worker {
    key: WorkerKey,
    child: Child,
    input: Option<mpsc::SyncSender<Vec<Arc<Frame>>>>,
    packets: mpsc::Receiver<std::io::Result<Vec<u8>>>,
    writer: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
}
impl Worker {
    fn start(exe: &Path, frames: &[Arc<Frame>], o: &AvifOptions) -> Result<Self> {
        let key = WorkerKey::new(frames, o);
        let mut cmd = command(exe);
        let crf = ((100 - u32::from(o.quality)) * 63 + 50) / 100;
        cmd.args([
            "-v",
            "error",
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-s",
            &format!("{}x{}", key.width, key.height),
            "-r",
            "24",
            "-i",
            "pipe:0",
            "-an",
            "-vf",
            "scale=out_range=full,format=yuv420p",
            "-color_range",
            "pc",
            "-colorspace",
            "smpte170m",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "iec61966-2-1",
            "-c:v",
            "libaom-av1",
            "-usage",
            if o.speed == EncodeSpeed::Ahead {
                "good"
            } else {
                "realtime"
            },
            "-cpu-used",
            if o.speed == EncodeSpeed::Ahead {
                "6"
            } else {
                "8"
            },
            "-crf",
            &crf.to_string(),
            "-b:v",
            if o.speed == EncodeSpeed::Ahead {
                "0"
            } else {
                "256k"
            },
            "-lag-in-frames",
            if o.speed == EncodeSpeed::Ahead {
                "25"
            } else {
                "0"
            },
            "-auto-alt-ref",
            if o.speed == EncodeSpeed::Ahead {
                "1"
            } else {
                "0"
            },
            "-g",
            &frames.len().to_string(),
            "-threads",
            &o.threads.to_string(),
            "-row-mt",
            "1",
            "-tune",
            "ssim",
            "-f",
            "ivf",
            "-flush_packets",
            "1",
        ]);
        if o.speed != EncodeSpeed::Ahead {
            cmd.args([
                "-minrate",
                "256k",
                "-maxrate",
                "256k",
                "-qmin",
                &crf.saturating_sub(4).to_string(),
                "-qmax",
                &(crf + 4).min(63).to_string(),
                "-aom-params",
                if o.speed == EncodeSpeed::Fast {
                    "cpu-used=9"
                } else {
                    "cpu-used=8"
                },
            ]);
        }
        cmd.arg("pipe:1");
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let mut stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = mpsc::sync_channel::<Vec<Arc<Frame>>>(1);
        let writer = thread::spawn(move || {
            while let Ok(frames) = rx.recv() {
                for f in frames {
                    if stdin.write_all(f.pixels()).is_err() {
                        return;
                    }
                }
                if stdin.flush().is_err() {
                    return;
                }
            }
        });
        let (ptx, prx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let read = |stdout: &mut std::process::ChildStdout| -> std::io::Result<Vec<u8>> {
                let mut h = [0; 12];
                stdout.read_exact(&mut h)?;
                let n = u32::from_le_bytes(h[..4].try_into().expect("4 bytes")) as usize;
                if n > 64 * 1024 * 1024 {
                    return Err(std::io::Error::other("oversize AV1 packet"));
                }
                let mut data = vec![0; n];
                stdout.read_exact(&mut data)?;
                Ok(data)
            };
            let mut header = [0; 32];
            if let Err(e) = stdout.read_exact(&mut header) {
                let _ = ptx.send(Err(e));
                return;
            }
            if &header[..4] != b"DKIF" || &header[8..12] != b"AV01" {
                let _ = ptx.send(Err(std::io::Error::other("invalid IVF header")));
                return;
            }
            loop {
                let packet = read(&mut stdout);
                let failed = packet.is_err();
                if ptx.send(packet).is_err() || failed {
                    return;
                }
            }
        });
        Ok(Self {
            key,
            child,
            input: Some(tx),
            packets: prx,
            writer: Some(writer),
            reader: Some(reader),
        })
    }
    fn encode(&mut self, frames: &[Arc<Frame>], o: &AvifOptions) -> Result<Vec<u8>> {
        self.input
            .as_ref()
            .ok_or(Error::Protocol("closed AVIF worker"))?
            .send(frames.to_vec())
            .map_err(|_| Error::Protocol("AVIF input worker exited"))?;
        if o.speed == EncodeSpeed::Ahead {
            self.input.take();
        }
        let mut packets = Vec::with_capacity(frames.len());
        for _ in frames {
            packets.push(
                self.packets
                    .recv_timeout(Duration::from_secs(30))
                    .map_err(|_| Error::Protocol("AVIF worker timed out or exited"))??,
            );
        }
        avif_mux::mux(
            &packets,
            self.key.width,
            self.key.height,
            &o.durations_ms,
            o.plays,
        )
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.input.take();
        let deadline = std::time::Instant::now() + Duration::from_millis(150);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                _ if std::time::Instant::now() >= deadline => {
                    terminate(&mut self.child);
                    break;
                }
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        if let Some(h) = self.writer.take() {
            let _ = h.join();
        }
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
    }
}

/// Experimental installed libaom through persistent ffmpeg IVF pipes, muxed in-process as AVIF.
/// Warm encodes are fast, but measured matched-size quality trails the native backend.
/// Keep one instance per encode worker. At most two processes retain the video/glow configurations;
/// changing quality replaces the least recently used configuration. Drop stops and joins both.
pub struct PersistentAvifEncoder {
    ffmpeg: PathBuf,
    workers: Mutex<Vec<Worker>>,
}
impl PersistentAvifEncoder {
    /// Lazy startup; call `encode` once for video and glow to warm a worker before timed playback.
    pub fn new(ffmpeg: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
            workers: Mutex::new(Vec::new()),
        }
    }
}
impl AvifEncoder for PersistentAvifEncoder {
    fn encode(&self, frames: &[Arc<Frame>], o: &AvifOptions) -> Result<Vec<u8>> {
        validate(frames, o)?;
        if o.speed == EncodeSpeed::Ahead {
            return Worker::start(&self.ffmpeg, frames, o)?.encode(frames, o);
        }
        let key = WorkerKey::new(frames, o);
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| Error::Protocol("AVIF worker mutex poisoned"))?;
        let mut worker = if let Some(i) = workers.iter().position(|w| w.key == key) {
            workers.remove(i)
        } else {
            if workers.len() == 2 {
                workers.remove(0);
            }
            Worker::start(&self.ffmpeg, frames, o)?
        };
        let result = worker.encode(frames, o);
        if result.is_ok() {
            workers.push(worker);
        }
        result
    }
}

/// Millisecond durations with ties-to-even cumulative rounding, matching Python `round`.
pub fn frame_durations(count: usize, fps: u32) -> Result<Vec<u32>> {
    if fps == 0 || fps > 1000 {
        return Err(Error::InvalidInput("frame rate must be 1..1000"));
    }
    let rounded = |n: u128| -> u128 {
        let d = u128::from(fps);
        let q = n / d;
        let r = n % d;
        q + u128::from(r * 2 > d || (r * 2 == d && !q.is_multiple_of(2)))
    };
    Ok((0..count)
        .map(|i| (rounded((i as u128 + 1) * 1000) - rounded(i as u128 * 1000)) as u32)
        .collect())
}
/// Decode a local clip; fast uses q55/speed10, normal uses q60/speed8, both with six threads.
/// This belongs on the dedicated favicon worker, outside the write loop.
pub fn encode_clip_svg_with(
    encoder: &dyn AvifEncoder,
    ffmpeg: &Path,
    request: &FaviconEncodeRequest,
    fast: bool,
) -> Result<EncodedFavicon> {
    if request.duration.is_zero() || !request.output.is_absolute() {
        return Err(Error::InvalidInput(
            "favicon needs positive duration and absolute output",
        ));
    }
    let mut child = command(ffmpeg)
        .args([
            "-v",
            "error",
            "-ss",
            &format!("{:.3}", request.start.as_secs_f64()),
            "-i",
        ])
        .arg(&request.video)
        .args([
            "-an",
            "-t",
            &format!("{:.3}", request.duration.as_secs_f64()),
            "-vf",
            "fps=24,scale=640:360:force_original_aspect_ratio=increase:flags=area,crop=640:360",
            "-threads",
            "2",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped decode");
    let mut frames = Vec::new();
    let expected = (request.duration.as_secs_f64() * 24.0).ceil() as usize + 1;
    let read = (|| -> Result<()> {
        loop {
            let mut bytes = vec![0; 640 * 360 * 3];
            let mut n = 0;
            while n < bytes.len() {
                let got = stdout.read(&mut bytes[n..])?;
                if got == 0 {
                    if n != 0 {
                        return Err(Error::Protocol("partial decoded frame"));
                    }
                    return Ok(());
                }
                n += got;
            }
            frames.push(Arc::new(Frame::new(640, 360, bytes)?));
            if frames.len() > expected {
                return Err(Error::Protocol(
                    "favicon decoder exceeded requested duration",
                ));
            }
        }
    })();
    if let Err(error) = read {
        terminate(&mut child);
        return Err(error);
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(Error::ExternalTool {
            program: "ffmpeg",
            code: status.code(),
        });
    }
    if frames.is_empty() {
        return Err(Error::InvalidInput("no favicon frames decoded"));
    }
    let opts = AvifOptions {
        quality: if fast { 55 } else { 60 },
        speed: if fast {
            EncodeSpeed::Fast
        } else {
            EncodeSpeed::Favicon
        },
        durations_ms: frame_durations(frames.len(), 24)?,
        plays: 1,
        threads: 6,
    };
    let data = b64(&encoder.encode(&frames, &opts)?);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"640\" height=\"360\" viewBox=\"0 0 640 360\"><image href=\"data:image/avif;base64,{data}\" width=\"640\" height=\"360\"/>"
    );
    let padding = dt_core::constants::FAVICON_MIN_BYTES.saturating_sub(svg.len() + 6);
    if padding > 0 {
        svg.push_str("<!--");
        svg.extend(std::iter::repeat_n('.', padding));
        svg.push_str("-->");
    }
    svg.push_str("</svg>");
    std::fs::write(&request.output, svg)?;
    Ok(EncodedFavicon {
        path: request.output.clone(),
        frames: frames.len() as u64,
    })
}

#[path = "native.rs"]
mod native;
pub use native::NativeAvifEncoder;
