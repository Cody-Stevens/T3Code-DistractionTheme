//! Independent audio-clock scheduling, never blocked by card/glow/theme construction.
use crate::{
    favicon::FaviconDriver,
    player::{Player, SharedOutput},
    write_loop::WriteLoop,
};
use dt_core::{Clock, Error, Result, TrackInfo};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct Input {
    pub(crate) track: Option<TrackInfo>,
    pub(crate) jumps: u64,
    pub(crate) playing: bool,
    pub(crate) enabled: bool,
    pub(crate) output_latency: Duration,
    revision: u64,
}
impl Input {
    fn player(p: &Player) -> Self {
        Self {
            track: p.ready().map(|t| t.info.clone()),
            jumps: p.jumps,
            playing: p.playing,
            enabled: p.favicon_enabled,
            output_latency: p.output_latency,
            revision: p.revision,
        }
    }
}
pub(crate) trait Host {
    fn input(&self) -> Input;
    fn clock(&self) -> Arc<dyn Clock>;
    fn hold_audio(&mut self, deadline: Instant) -> Result<Duration>;
    fn release_audio(&mut self, at: Option<Instant>) -> Result<()>;
    fn quiet_landed(&self) -> Option<Instant>;
    fn hold_until(&self) -> Option<Instant>;
    fn request_quiet(&mut self);
    fn set_front(&mut self, front: Option<char>);
    fn set_lag(&mut self, lag: f64);
    fn set_allowed(&mut self, files: Vec<String>);
    fn set_live(&mut self, live: bool);
}
pub(crate) struct OwnerHost<'a> {
    pub(crate) player: &'a mut Player,
    pub(crate) writes: &'a mut WriteLoop,
}
impl Host for OwnerHost<'_> {
    fn input(&self) -> Input {
        Input::player(self.player)
    }
    fn clock(&self) -> Arc<dyn Clock> {
        self.player.clock()
    }
    fn hold_audio(&mut self, deadline: Instant) -> Result<Duration> {
        self.player.hold_audio(deadline)
    }
    fn release_audio(&mut self, at: Option<Instant>) -> Result<()> {
        self.player.release_audio(at)
    }
    fn quiet_landed(&self) -> Option<Instant> {
        self.writes.quiet_written_at()
    }
    fn hold_until(&self) -> Option<Instant> {
        self.writes.timing.hold_until
    }
    fn request_quiet(&mut self) {
        self.writes.request_quiet();
    }
    fn set_front(&mut self, front: Option<char>) {
        apply(self.player, Presentation::Front(front));
    }
    fn set_lag(&mut self, lag: f64) {
        apply(self.player, Presentation::Lag(lag));
    }
    fn set_allowed(&mut self, files: Vec<String>) {
        apply(self.player, Presentation::Allowed(files));
    }
    fn set_live(&mut self, live: bool) {
        apply(self.player, Presentation::Live(live));
    }
}
enum Presentation {
    Front(Option<char>),
    Lag(f64),
    Allowed(Vec<String>),
    Live(bool),
    Wake,
}
fn apply(p: &mut Player, change: Presentation) {
    match change {
        Presentation::Front(front) => {
            p.favicon_front = front;
        }
        Presentation::Lag(lag) => {
            p.favicon_lag = lag;
        }
        Presentation::Allowed(files) => {
            p.favicon_allow = Some(files);
        }
        Presentation::Live(live) => {
            p.favicon_live = live;
            if !live {
                p.favicon_front = None;
                p.favicon_lag = 0.0;
            }
        }
        Presentation::Wake => (),
    }
    p.dirty = true;
}
struct Shared {
    input: Input,
    quiet_requested: Option<u64>,
    landed: Option<Instant>,
    hold_until: Option<Instant>,
}
struct ThreadHost {
    input: Input,
    shared: Arc<Mutex<Shared>>,
    audio: SharedOutput,
    output: mpsc::Sender<(u64, Presentation)>,
}
impl ThreadHost {
    fn send(&self, output: Presentation) {
        let _ = self.output.send((self.input.revision, output));
    }
}
impl Host for ThreadHost {
    fn input(&self) -> Input {
        self.input.clone()
    }
    fn clock(&self) -> Arc<dyn Clock> {
        Arc::new(self.audio.clone())
    }
    fn hold_audio(&mut self, deadline: Instant) -> Result<Duration> {
        let pos = self.audio.hold(deadline)?;
        self.send(Presentation::Wake);
        Ok(pos)
    }
    fn release_audio(&mut self, at: Option<Instant>) -> Result<()> {
        self.audio.release(at)?;
        self.send(Presentation::Wake);
        Ok(())
    }
    fn quiet_landed(&self) -> Option<Instant> {
        let shared = self.shared.lock().expect("favicon state");
        (shared.input.revision == self.input.revision)
            .then_some(shared.landed)
            .flatten()
    }
    fn hold_until(&self) -> Option<Instant> {
        let shared = self.shared.lock().expect("favicon state");
        (shared.input.revision == self.input.revision)
            .then_some(shared.hold_until)
            .flatten()
    }
    fn request_quiet(&mut self) {
        let mut shared = self.shared.lock().expect("favicon state");
        if shared.input.revision == self.input.revision {
            shared.quiet_requested = Some(self.input.revision);
        }
    }
    fn set_front(&mut self, front: Option<char>) {
        self.send(Presentation::Front(front));
    }
    fn set_lag(&mut self, lag: f64) {
        self.send(Presentation::Lag(lag));
    }
    fn set_allowed(&mut self, files: Vec<String>) {
        self.send(Presentation::Allowed(files));
    }
    fn set_live(&mut self, live: bool) {
        self.send(Presentation::Live(live));
    }
}
/// Own the favicon driver and effects on a dedicated thread, like Python's FaviconVideo._loop.
pub struct FaviconThread {
    shared: Arc<Mutex<Shared>>,
    output: mpsc::Receiver<(u64, Presentation)>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<()>>>,
}
impl FaviconThread {
    /// Start with a source snapshot and the real independently synchronized output handle.
    pub fn start(mut driver: FaviconDriver, player: &Player) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared {
            input: Input::player(player),
            quiet_requested: None,
            landed: None,
            hold_until: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let state = shared.clone();
        let audio = player.audio_handle();
        let (send, output) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("distraction-favicon".into())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    let input = state.lock().expect("favicon state").input.clone();
                    let mut host = ThreadHost {
                        input,
                        shared: state.clone(),
                        audio: audio.clone(),
                        output: send.clone(),
                    };
                    if let Err(error) = driver.tick_host(&mut host, Instant::now()) {
                        eprintln!("favicon tick failed: {error}");
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                let release = audio.release(None);
                let stopped = driver.stop();
                release.and(stopped)
            })?;
        Ok(Self {
            shared,
            output,
            stop,
            thread: Some(thread),
        })
    }
    /// Publish discontinuities/grants and consume presentation changes without waiting on codecs.
    pub fn synchronize(&mut self, player: &mut Player, writes: &mut WriteLoop) {
        let mut shared = self.shared.lock().expect("favicon state");
        if shared.input.revision != player.revision {
            shared.quiet_requested = None;
            // A restart may have been observed from the new audio time before the owner
            // published its jump counter. Its old-revision outputs are discarded below;
            // clear presentation here too so a stale back-deck opacity cannot hide A.
            apply(player, Presentation::Live(false));
            apply(player, Presentation::Allowed(Vec::new()));
        }
        shared.input = Input::player(player);
        shared.landed = writes.quiet_written_at();
        shared.hold_until = writes.timing.hold_until;
        if shared.quiet_requested.take() == Some(player.revision) {
            writes.request_quiet();
        }
        drop(shared);
        for (revision, change) in self.output.try_iter() {
            if revision == player.revision {
                apply(player, change);
            }
        }
    }
    /// Release any audio hold, cancel owned encoding, blank decks and join the scheduler.
    pub fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| Error::Protocol("favicon thread failed"))??;
        }
        Ok(())
    }
}
impl Drop for FaviconThread {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
