//! Start/stop/restart supervision from `service.py`, with an injectable child/runtime host.
use dt_core::{Result, constants::*};
use dt_t3::T3Paths;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// Read only the runtime PID and require process liveness; missing/invalid JSON means stopped.
pub fn runtime_running(runtime: &Path, alive: impl FnOnce(u32) -> bool) -> bool {
    let pid = fs::read(runtime)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.get("pid").and_then(|pid| pid.as_u64()))
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0);
    pid.is_some_and(alive)
}
/// Poll T3's actual PID; the runtime file's existence alone is insufficient.
pub fn t3_running(paths: &T3Paths) -> bool {
    runtime_running(&paths.runtime_file, dt_platform::process::pid_alive)
}

/// Spawn this executable with play, hidden, append-only logs, without a shell.
pub fn start_player(executable: &Path, state_dir: &Path) -> Result<Child> {
    fs::create_dir_all(state_dir)?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(state_dir.join("player.log"))?;
    let mut command = Command::new(executable);
    command
        .arg("play")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    dt_platform::process::spawn_hidden(&mut command)
}
/// Injectable owned-child shutdown transport and clock.
pub trait ChildShutdown {
    /// Reap an exited child; its owned descendants still need cleanup.
    fn exited(&mut self) -> Result<bool>;
    /// Attempt ctl quit with the given timeout; do not target unrelated helpers.
    fn quit(&mut self, timeout: Duration) -> bool;
    /// Elapsed monotonic time.
    fn now(&self) -> Duration;
    /// Bounded wait (fake implementations advance their clock).
    fn wait(&mut self, duration: Duration);
    /// Kill and reap the whole owned process tree.
    fn kill_tree(&mut self) -> Result<()>;
}
/// Graceful quit (3 s request), up to 10 s exit wait, then release the whole owned tree.
pub fn stop_owned_child(child: &mut dyn ChildShutdown) -> Result<()> {
    if child.exited()? {
        return child.kill_tree();
    }
    if child.quit(Duration::from_secs(3)) {
        let deadline = child.now() + Duration::from_secs(10);
        loop {
            if child.exited()? {
                break;
            }
            let remaining = deadline.saturating_sub(child.now());
            if remaining.is_zero() {
                break;
            }
            child.wait(remaining.min(Duration::from_millis(50)));
        }
    }
    child.kill_tree()
}
struct NativeShutdown<'a> {
    child: &'a mut Child,
    port: u16,
    started: Instant,
}
impl ChildShutdown for NativeShutdown<'_> {
    fn exited(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_some())
    }
    fn quit(&mut self, timeout: Duration) -> bool {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build();
        client.is_ok_and(|client| {
            client
                .get(format!("http://127.0.0.1:{}/quit", self.port))
                .send()
                .is_ok_and(|response| response.status().is_success())
        })
    }
    fn now(&self) -> Duration {
        self.started.elapsed()
    }
    fn wait(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
    fn kill_tree(&mut self) -> Result<()> {
        dt_platform::process::kill_process_tree(self.child)
    }
}
/// Stop only the Child handle owned by this supervisor, never a discovered/foreign PID.
pub fn stop_player(child: &mut Child, port: u16) -> Result<()> {
    stop_owned_child(&mut NativeShutdown {
        child,
        port,
        started: Instant::now(),
    })
}
/// Process effects; fake hosts exercise the complete restart state machine.
pub trait SupervisorHost {
    /// Re-read runtime and check PID liveness.
    fn running(&mut self) -> bool;
    /// Explicit setup before each start.
    fn setup(&mut self) -> Result<()>;
    /// Start one owned player.
    fn start(&mut self) -> Result<()>;
    /// Whether the owned player exited; reap it when true.
    fn exited(&mut self) -> Result<bool>;
    /// Request quit then kill/reap the whole owned child tree if needed.
    fn stop(&mut self) -> Result<()>;
    /// Append a static, non-sensitive event message.
    fn log(&mut self, message: &str);
}
/// Restart policy, independent of sleeping, live files, and Win32 processes.
pub struct Supervisor<H: SupervisorHost> {
    /// Host retained for deterministic tests/embedding.
    pub host: H,
    child: bool,
    crashes: u32,
    next_start: Duration,
    started_at: Duration,
}
impl<H: SupervisorHost> Supervisor<H> {
    /// No process or filesystem side effects.
    pub fn new(host: H) -> Self {
        Self {
            host,
            child: false,
            crashes: 0,
            next_start: Duration::ZERO,
            started_at: Duration::ZERO,
        }
    }
    /// One three-second poll; injected elapsed monotonic time controls back-off.
    pub fn tick(&mut self, now: Duration) -> Result<()> {
        let running = self.host.running();
        if self.child && !running {
            self.host.log("T3 closed; stopping the player");
            self.host.stop()?;
            self.child = false;
            self.crashes = 0;
            self.next_start = Duration::ZERO;
        } else if self.child && self.host.exited()? {
            self.child = false;
            if running {
                self.crashes = self.crashes.saturating_add(1);
                let delay = Duration::from_secs(2u64.saturating_pow(self.crashes).min(60));
                self.next_start = now + delay;
                self.host.log("player exited; restart scheduled");
            }
        } else if running && !self.child && now >= self.next_start {
            match self.host.setup().and_then(|()| self.host.start()) {
                Ok(()) => {
                    self.child = true;
                    self.started_at = now;
                    self.host.log("player started");
                }
                Err(_) => {
                    self.next_start = now + Duration::from_secs(30);
                    self.host.log("setup/start failed; retry in 30 seconds");
                }
            }
        } else if self.child
            && self.crashes > 0
            && now.saturating_sub(self.started_at) >= Duration::from_secs(300)
        {
            self.crashes = 0;
        }
        Ok(())
    }
    /// Stop only the child this supervisor owns.
    pub fn shutdown(&mut self) -> Result<()> {
        if self.child {
            self.host.stop()?;
            self.child = false;
        }
        Ok(())
    }
}
impl<H: SupervisorHost> Drop for Supervisor<H> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
struct NativeHost {
    paths: T3Paths,
    executable: PathBuf,
    state: PathBuf,
    child: Option<Child>,
    port: u16,
}
impl SupervisorHost for NativeHost {
    fn running(&mut self) -> bool {
        t3_running(&self.paths)
    }
    fn setup(&mut self) -> Result<()> {
        crate::runtime::setup_with_paths(self.paths.clone())
    }
    fn start(&mut self) -> Result<()> {
        // Do not supervise a competing helper already listening on this port.
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, self.port))?;
        drop(listener);
        self.child = Some(start_player(&self.executable, &self.state)?);
        Ok(())
    }
    fn exited(&mut self) -> Result<bool> {
        if self
            .child
            .as_mut()
            .map(|child| child.try_wait())
            .transpose()?
            .flatten()
            .is_some()
        {
            dt_platform::process::kill_process_tree(self.child.as_mut().expect("owned child"))?;
            self.child.take();
            return Ok(true);
        }
        Ok(false)
    }
    fn stop(&mut self) -> Result<()> {
        if let Some(child) = self.child.as_mut() {
            stop_player(child, self.port)?;
        }
        self.child.take();
        Ok(())
    }
    fn log(&mut self, message: &str) {
        if fs::create_dir_all(&self.state).is_ok()
            && let Ok(mut log) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.state.join("service.log"))
        {
            let _ = writeln!(log, "{message}");
        }
        eprintln!("{message}");
    }
}
/// Hold one service mutex, poll every 3 s, and stop the owned player on shutdown/T3 exit.
pub fn serve() -> Result<()> {
    let Some(_instance) =
        dt_platform::process::single_instance(dt_platform::process::SERVICE_MUTEX)?
    else {
        return Ok(());
    };
    let shutdown = crate::runtime::termination_flag()?;
    let host = NativeHost {
        paths: T3Paths::discover()?,
        executable: std::env::current_exe()?,
        state: crate::runtime::state_dir()?,
        child: None,
        port: crate::runtime::control_port()?,
    };
    let mut supervisor = Supervisor::new(host);
    let start = Instant::now();
    while !shutdown.load(std::sync::atomic::Ordering::Acquire) {
        supervisor.tick(start.elapsed())?;
        let deadline = Instant::now() + SERVICE_POLL;
        while Instant::now() < deadline && !shutdown.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    supervisor.shutdown()
}
