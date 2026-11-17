//! Bounded look-ahead pipe, matching `glowtube.VideoReader`'s seek and loop behavior.

use crate::process::{OwnedChild, command};
use dt_core::{
    Error, Frame, Result,
    constants::{FPS, FRAME_H, FRAME_W},
};
use std::{
    collections::BTreeMap,
    io::Read,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
};

const RETAIN: u64 = FPS as u64 * 2;

/// Input and executable for a `VideoReader`; decode size and FPS come from dt-core.
#[derive(Clone, Debug)]
pub struct VideoReaderConfig {
    /// Source media file (local or downloaded).
    pub path: PathBuf,
    /// ffmpeg executable for RGB24 rawvideo output.
    pub ffmpeg: PathBuf,
}

/// Nonblocking frame lookup; seeks restart the pipe without waiting for a decoded frame.
pub trait VideoSource: Send {
    /// Requested/nearest older RGB24 frame, or `None` while loading or restarting.
    fn get(&mut self, frame: u64) -> Result<Option<Arc<Frame>>>;
    /// Restart at a 24 fps frame index and invalidate the old buffer.
    fn start_at(&mut self, frame: u64) -> Result<()>;
    /// Terminate/reap the owned process and join its worker; also called on drop.
    fn stop(&mut self) -> Result<()>;
}

#[derive(Default)]
struct State {
    frames: BTreeMap<u64, Arc<Frame>>,
    base: u64,
    decoded: u64,
    wanted: u64,
    stopped: bool,
    eof: bool,
    failed: bool,
}

/// Aspect-fill 640x360 RGB24 with two seconds of history and about two seconds of look-ahead.
/// A loop restarts whenever the request precedes the oldest retained frame, even near EOF.
pub struct VideoReader {
    config: VideoReaderConfig,
    shared: Arc<(Mutex<State>, Condvar)>,
    child: Option<OwnedChild>,
    worker: Option<JoinHandle<()>>,
}

fn pump(mut pipe: impl Read, shared: Arc<(Mutex<State>, Condvar)>) {
    let size = FRAME_W as usize * FRAME_H as usize * 3;
    loop {
        let mut bytes = vec![0; size];
        let mut filled = 0;
        let mut failed = false;
        while filled < size {
            match pipe.read(&mut bytes[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        let (lock, ready) = &*shared;
        let Ok(mut state) = lock.lock() else {
            return;
        };
        if filled != size {
            state.eof = true;
            state.failed = failed || filled != 0;
            return;
        }
        while !state.stopped && state.decoded > state.wanted.saturating_add(RETAIN) {
            let Ok(next) = ready.wait(state) else {
                return;
            };
            state = next;
        }
        if state.stopped {
            return;
        }
        let index = state.decoded;
        let Ok(frame) = Frame::new(FRAME_W, FRAME_H, bytes) else {
            state.failed = true;
            return;
        };
        state.frames.insert(index, Arc::new(frame));
        state.decoded = state.decoded.saturating_add(1);
        let oldest = state.wanted.saturating_sub(RETAIN);
        while state
            .frames
            .first_key_value()
            .is_some_and(|(&index, _)| index < oldest)
        {
            state.frames.pop_first();
        }
    }
}

impl VideoReader {
    /// Start decoding frame zero on a worker; no frame is required before this returns.
    pub fn open(config: VideoReaderConfig) -> Result<Self> {
        let mut reader = Self {
            config,
            shared: Arc::default(),
            child: None,
            worker: None,
        };
        reader.start_at(0)?;
        Ok(reader)
    }
}

impl VideoSource for VideoReader {
    fn get(&mut self, frame: u64) -> Result<Option<Arc<Frame>>> {
        let restart;
        {
            let (lock, ready) = &*self.shared;
            let mut state = lock
                .lock()
                .map_err(|_| Error::Protocol("video buffer lock"))?;
            if state.stopped {
                return Ok(None);
            }
            state.wanted = frame;
            ready.notify_one();
            if state.failed {
                return Err(Error::Protocol("incomplete ffmpeg video frame"));
            }
            if state.eof
                && let Some(child) = &mut self.child
                && let Some(status) = child.0.try_wait()?
            {
                crate::process::check(status, "ffmpeg")?;
            }
            if let Some(hit) = state.frames.get(&frame) {
                return Ok(Some(Arc::clone(hit)));
            }
            let oldest = state
                .frames
                .first_key_value()
                .map_or(state.base, |(&k, _)| k);
            restart = frame < oldest || frame > state.decoded.saturating_add(RETAIN);
            if !restart {
                return Ok(state
                    .frames
                    .range(..=frame)
                    .next_back()
                    .map(|(_, image)| Arc::clone(image)));
            }
        }
        if restart {
            self.start_at(frame)?;
        }
        Ok(None)
    }

    fn start_at(&mut self, frame: u64) -> Result<()> {
        self.stop()?;
        let shared = Arc::new((
            Mutex::new(State {
                base: frame,
                decoded: frame,
                wanted: frame,
                ..State::default()
            }),
            Condvar::new(),
        ));
        let filter = format!(
            "fps={FPS},scale={FRAME_W}:{FRAME_H}:force_original_aspect_ratio=increase:flags=area,crop={FRAME_W}:{FRAME_H}"
        );
        let mut child = OwnedChild(
            command(&self.config.ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-ss",
                    &format!("{:.3}", frame as f64 / f64::from(FPS)),
                    "-i",
                ])
                .arg(&self.config.path)
                .args([
                    "-an", "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "rgb24", "-",
                ])
                .stdout(Stdio::piped())
                .spawn()?,
        );
        let pipe = child
            .0
            .stdout
            .take()
            .ok_or(Error::Protocol("ffmpeg stdout"))?;
        let worker_state = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("dt-video".into())
            .spawn(move || pump(pipe, worker_state))?;
        self.shared = shared;
        self.child = Some(child);
        self.worker = Some(worker);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        let (lock, ready) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.stopped = true;
            ready.notify_all();
        }
        let result = self
            .child
            .take()
            .map_or(Ok(()), |mut child| child.terminate().map_err(Error::from));
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| Error::Protocol("video worker panicked"))?;
        }
        if let Ok(mut state) = lock.lock() {
            state.frames.clear();
        }
        result
    }
}

impl Drop for VideoReader {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct Frames {
        bytes_left: usize,
    }
    impl Read for Frames {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let count = out.len().min(self.bytes_left);
            out[..count].fill(17);
            self.bytes_left -= count;
            Ok(count)
        }
    }

    fn until(shared: &Arc<(Mutex<State>, Condvar)>, condition: impl Fn(&State) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition(&shared.0.lock().unwrap()) {
            assert!(Instant::now() < deadline, "video pump timed out");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn pump_bounds_ahead_and_retains_two_seconds_of_history() {
        let shared: Arc<(Mutex<State>, Condvar)> = Arc::default();
        let state = Arc::clone(&shared);
        let worker = thread::spawn(move || {
            pump(
                Frames {
                    bytes_left: 160 * FRAME_W as usize * FRAME_H as usize * 3,
                },
                state,
            )
        });
        until(&shared, |s| s.decoded == RETAIN + 1);
        {
            let mut s = shared.0.lock().unwrap();
            s.wanted = 100;
            shared.1.notify_one();
        }
        until(&shared, |s| s.decoded == 149);
        {
            let mut s = shared.0.lock().unwrap();
            assert_eq!(s.frames.first_key_value().unwrap().0, &52);
            assert_eq!(s.frames.last_key_value().unwrap().0, &148);
            assert_eq!(s.frames.len(), 97);
            s.stopped = true;
            shared.1.notify_one();
        }
        worker.join().unwrap();
    }

    #[test]
    fn incomplete_pipe_frames_are_rejected() {
        let shared: Arc<(Mutex<State>, Condvar)> = Arc::default();
        pump(std::io::Cursor::new([0; 3]), Arc::clone(&shared));
        let state = shared.0.lock().unwrap();
        assert!(state.eof && state.failed);
        assert!(state.frames.is_empty());
    }
}
