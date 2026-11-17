//! Owned, warmed favicon process. Killing its Job Object interrupts libavif and its decoder.
use crate::favicon::EncodeEpoch;
use dt_core::{Error, Frame, Result};
use dt_render::encode::{AvifEncoder, AvifOptions, EncodeSpeed, NativeAvifEncoder};
use dt_t3::favicon::{ClipJob, ReadyClip};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Serialize, Deserialize)]
struct Task {
    ffmpeg: PathBuf,
    video: PathBuf,
    start: Duration,
    duration: Duration,
    output: PathBuf,
    fast: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Reply {
    Ready,
    Encoded { frames: u64 },
    Failed,
}
type Pipes = (ChildStdin, BufReader<ChildStdout>);
struct Active {
    child: Child,
    pipes: Option<Pipes>,
}
/// One persistent child per favicon worker, with independent cancellation access to its tree.
pub struct EncoderProcess {
    executable: PathBuf,
    ffmpeg: PathBuf,
    active: Mutex<Option<Active>>,
}
impl EncoderProcess {
    /// No process starts until the encode worker asks for warmup.
    pub fn new(executable: PathBuf, ffmpeg: PathBuf) -> Self {
        Self {
            executable,
            ffmpeg,
            active: Mutex::new(None),
        }
    }
    /// Start and warm the codec outside the write owner. Cancellation also interrupts this read.
    pub fn warm(&self, epoch: &EncodeEpoch) -> Result<()> {
        let (id, mut pipes) = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| Error::Protocol("encoder lock failed"))?;
            if !epoch.current() {
                return Err(Error::Protocol("obsolete favicon encode"));
            }
            if active.is_some() {
                return Ok(());
            }
            let mut command = Command::new(&self.executable);
            command
                .arg("internal-encode-favicon")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let mut child = dt_platform::process::spawn_hidden(&mut command)?;
            let pipes = (
                child.stdin.take().expect("piped input"),
                BufReader::new(child.stdout.take().expect("piped output")),
            );
            let id = child.id();
            *active = Some(Active { child, pipes: None });
            (id, pipes)
        };
        let result = match read_reply(&mut pipes.1) {
            Ok(Reply::Ready) => Ok(()),
            _ => Err(Error::Protocol("favicon encoder warmup failed")),
        };
        self.restore(id, pipes)?;
        if result.is_err() {
            self.cancel()?;
        }
        result
    }
    /// Send one bounded request and read one small result; frames stay in the owned process.
    pub fn encode(&self, job: ClipJob, epoch: &EncodeEpoch) -> Result<ReadyClip> {
        self.warm(epoch)?;
        let (id, mut pipes) = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| Error::Protocol("encoder lock failed"))?;
            if !epoch.current() {
                return Err(Error::Protocol("obsolete favicon encode"));
            }
            let active = active
                .as_mut()
                .ok_or(Error::Protocol("encoder was replaced"))?;
            (
                active.child.id(),
                active
                    .pipes
                    .take()
                    .ok_or(Error::Protocol("encoder already busy"))?,
            )
        };
        let result = (|| {
            let fast = match (job.speed, job.quality) {
                (10, 55) => true,
                (8, 60) => false,
                _ => return Err(Error::InvalidInput("unsupported favicon quality intent")),
            };
            let task = Task {
                ffmpeg: self.ffmpeg.clone(),
                video: job.video.clone(),
                start: job.start,
                duration: job.duration,
                output: job.output.clone(),
                fast,
            };
            let bytes = serde_json::to_vec(&task)
                .map_err(|_| Error::Protocol("invalid encoder request"))?;
            if bytes.len() > 32768 {
                return Err(Error::InvalidInput("encoder request too large"));
            }
            pipes.0.write_all(&bytes)?;
            pipes.0.write_all(b"\n")?;
            pipes.0.flush()?;
            match read_reply(&mut pipes.1)? {
                Reply::Encoded { frames } if frames > 0 && epoch.current() => {
                    Ok(ReadyClip { job, frames })
                }
                _ => Err(Error::Protocol("favicon encode failed or cancelled")),
            }
        })();
        self.restore(id, pipes)?;
        if result.is_err() {
            self.cancel()?;
        }
        result
    }
    fn restore(&self, id: u32, pipes: Pipes) -> Result<()> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| Error::Protocol("encoder lock failed"))?;
        if let Some(active) = active.as_mut().filter(|a| a.child.id() == id) {
            active.pipes = Some(pipes);
        }
        Ok(())
    }
    /// Kill/reap the whole child tree before a replacement can take its place.
    pub fn cancel(&self) -> Result<()> {
        let active = self
            .active
            .lock()
            .map_err(|_| Error::Protocol("encoder lock failed"))?
            .take();
        if let Some(mut active) = active {
            dt_platform::process::kill_process_tree(&mut active.child)?;
        }
        Ok(())
    }
}
impl Drop for EncoderProcess {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}
fn read_reply(reader: &mut BufReader<ChildStdout>) -> Result<Reply> {
    let mut bytes = Vec::new();
    reader.by_ref().take(1025).read_until(b'\n', &mut bytes)?;
    if bytes.len() > 1024 {
        return Err(Error::Protocol("encoder response too large"));
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::Protocol("invalid encoder response"))
}
fn reply(output: &mut impl Write, value: &Reply) -> Result<()> {
    serde_json::to_writer(&mut *output, value)
        .map_err(|_| Error::Protocol("encoder reply failed"))?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
/// Hidden subprocess entry point: warm libavif, then decode/encode only the supplied local jobs.
/// It never discovers T3 paths, opens devices, reads tokens or binds a control port.
pub fn serve() -> Result<()> {
    NativeAvifEncoder.encode(
        &[Arc::new(Frame::new(16, 16, vec![0; 16 * 16 * 3])?)],
        &AvifOptions {
            quality: 50,
            speed: EncodeSpeed::Fast,
            durations_ms: vec![42],
            plays: 1,
            threads: 1,
        },
    )?;
    let mut output = std::io::stdout().lock();
    reply(&mut output, &Reply::Ready)?;
    let mut input = std::io::stdin().lock();
    loop {
        let mut bytes = Vec::new();
        input.by_ref().take(32770).read_until(b'\n', &mut bytes)?;
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() > 32769 {
            return Err(Error::InvalidInput("encoder request too large"));
        }
        let task: Task = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Protocol("invalid encoder request"))?;
        let request = dt_render::encode::FaviconEncodeRequest {
            video: task.video,
            start: task.start,
            duration: task.duration,
            output: task.output,
        };
        let result = dt_render::encode::encode_clip_svg_with(
            &NativeAvifEncoder,
            &task.ffmpeg,
            &request,
            task.fast,
        );
        reply(
            &mut output,
            &match result {
                Ok(encoded) => Reply::Encoded {
                    frames: encoded.frames,
                },
                Err(_) => Reply::Failed,
            },
        )?;
    }
}
