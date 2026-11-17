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
    time::{Duration, Instant},
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
    /// Pin retention to the first frame of a write before reading its frames with `get`.
    /// Keeps two seconds before `min(batch_start, requested_frame)` until the next batch.
    fn begin_batch(&mut self, frame: u64) -> Result<()>;
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
    keep: u64,
    started: Option<Instant>,
    stopped: bool,
    eof: bool,
    failed: bool,
}

impl State {
    fn restart_needed(&self, frame: u64, now: Instant) -> bool {
        let oldest = self.frames.first_key_value().map_or(self.base, |(&k, _)| k);
        let fresh = self
            .started
            .is_some_and(|started| now.saturating_duration_since(started) < Duration::from_secs(1))
            && frame.saturating_sub(self.base) <= u64::from(FPS) * 4;
        frame < oldest || (frame > self.decoded.saturating_add(RETAIN) && !fresh)
    }
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
        let oldest = state.keep.min(state.wanted).saturating_sub(RETAIN);
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
    fn begin_batch(&mut self, frame: u64) -> Result<()> {
        let mut state = self
            .shared
            .0
            .lock()
            .map_err(|_| Error::Protocol("video buffer lock"))?;
        state.keep = frame;
        Ok(())
    }

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
            restart = state.restart_needed(frame, Instant::now());
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
                keep: frame,
                started: Some(Instant::now()),
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
            s.keep = 100;
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

    #[test]
    fn batch_pattern_keeps_overlapping_writes_without_restarts() {
        let shared = Arc::new((
            Mutex::new(State {
                started: Some(Instant::now()),
                ..State::default()
            }),
            Condvar::new(),
        ));
        let worker_state = Arc::clone(&shared);
        let mut reader = VideoReader {
            // An unexpected restart fails immediately instead of launching an external tool.
            config: VideoReaderConfig {
                path: "unused".into(),
                ffmpeg: PathBuf::new(),
            },
            shared: Arc::clone(&shared),
            child: None,
            worker: None,
        };
        reader.worker = Some(thread::spawn(move || {
            pump(
                Frames {
                    bytes_left: 500 * FRAME_W as usize * FRAME_H as usize * 3,
                },
                worker_state,
            )
        }));
        let starts = std::iter::once(0)
            .chain((1..=6).map(|i| i * 6))
            .chain((1..=6).map(|i| 36 + i * 48));
        for start in starts {
            reader.begin_batch(start).unwrap();
            for frame in start..start + 60 {
                reader.get(frame).unwrap();
            }
            // Let the pipe fill after each complete write, just as it does between writes.
            until(&shared, |s| s.decoded == start + 60 + RETAIN);
            let (has_start, oldest, count) = {
                let state = shared.0.lock().unwrap();
                (
                    state.frames.contains_key(&start),
                    *state.frames.first_key_value().unwrap().0,
                    state.frames.len(),
                )
            };
            assert!(has_start, "batch {start} discarded its own first frame");
            assert_eq!(oldest, start.saturating_sub(RETAIN));
            assert!(count <= 60 + 2 * RETAIN as usize);
            assert!(
                Arc::ptr_eq(&reader.shared, &shared),
                "decoder restarted after its initial start"
            );
        }
        reader.stop().unwrap();
    }

    #[test]
    fn restart_grace_has_time_and_distance_bounds_and_never_blocks_backward_seeks() {
        let now = Instant::now();
        let state = State {
            base: 100,
            decoded: 100,
            started: Some(now),
            ..State::default()
        };
        assert!(!state.restart_needed(160, now));
        assert!(!state.restart_needed(196, now + Duration::from_millis(999)));
        assert!(state.restart_needed(197, now));
        assert!(state.restart_needed(160, now + Duration::from_secs(1)));
        assert!(state.restart_needed(99, now));
    }
}
