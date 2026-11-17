//! Lifecycle and ACL primitives from `service.py`. No functions invoke a shell with user arguments.

use dt_core::Result;
use std::{
    path::Path,
    process::{Child, Command},
};

/// Rust supervisor mutex. Distinct from Python's `Local\GlowTubeService` during migration.
pub const SERVICE_MUTEX: &str = "Local\\GlowTubeDistractionService";

/// Held named mutex. Dropping closes its handle; no thread owns/releases the mutex itself.
pub struct InstanceGuard {
    #[cfg(windows)]
    _handle: std::os::windows::io::OwnedHandle,
}

/// Create a named mutex without acquiring it; return None if it already exists.
/// Keep the guard throughout supervision. Tests must supply their own unique names.
pub fn single_instance(name: &str) -> Result<Option<InstanceGuard>> {
    #[cfg(windows)]
    {
        Ok(crate::win32::single_instance(name)?.map(|handle| InstanceGuard { _handle: handle }))
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        crate::unsupported()
    }
}

/// Query PROCESS_QUERY_LIMITED_INFORMATION / STILL_ACTIVE; zero, missing or inaccessible is false.
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        crate::win32::pid_alive(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

/// Spawn CREATE_NO_WINDOW, preserving the command's arguments, environment, cwd and stdio.
/// Starts suspended, assigns a non-inheritable kill-on-close Job Object, then resumes.
/// A monitor closes the job on root exit. Call `kill_process_tree` even after graceful root exit
/// when deterministic descendant cleanup is required. Dropping std's Child alone does not stop it.
pub fn spawn_hidden(command: &mut Command) -> Result<Child> {
    #[cfg(windows)]
    {
        jobs::spawn(command)
    }
    #[cfg(not(windows))]
    {
        let _ = command;
        crate::unsupported()
    }
}

/// Terminate/reap an owned child's job, waiting for descendants too. Idempotent after exit.
/// For a child created outside `spawn_hidden`, use hidden taskkill /T /F /PID as Python does.
/// Never pass a child you did not start. Graceful /quit and its timeout belong to the supervisor.
pub fn kill_process_tree(child: &mut Child) -> Result<()> {
    #[cfg(windows)]
    {
        jobs::kill(child)
    }
    #[cfg(not(windows))]
    {
        let _ = child;
        crate::unsupported()
    }
}

/// Replace the file/directory DACL with current-user full control and disable inheritance.
/// Set the owner to that user. Directory grants inherit to files/subdirectories. Refuse reparse
/// points and unsupported filesystems; propagate every error. Never read file contents.
/// The binary can forward `dt_t3::setup::TokenPermissions::restrict_to_owner` to this function.
pub fn restrict_to_owner(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        crate::win32::restrict_to_owner(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        crate::unsupported()
    }
}

#[cfg(windows)]
mod jobs {
    use super::*;
    use crate::win32;
    use dt_core::Error;
    use std::{
        collections::HashMap,
        os::windows::{io::OwnedHandle, process::CommandExt},
        sync::{Arc, Mutex, OnceLock},
        thread,
        time::{Duration, Instant},
    };

    // Owned handles are Send+Sync; no raw Win32 handle escapes the FFI boundary.
    struct Job {
        handle: OwnedHandle,
        root: OwnedHandle,
        monitor: Mutex<Option<thread::JoinHandle<()>>>,
    }
    type Jobs = HashMap<u32, Arc<Job>>;
    fn registry() -> &'static Mutex<Jobs> {
        static JOBS: OnceLock<Mutex<Jobs>> = OnceLock::new();
        JOBS.get_or_init(Mutex::default)
    }
    fn lock() -> std::sync::MutexGuard<'static, Jobs> {
        registry().lock().unwrap_or_else(|error| error.into_inner())
    }
    fn key(child: &Child) -> u32 {
        // The monitor retains a process handle, so its PID cannot be reused while registered.
        // A Child's original numeric HANDLE can be recycled if the caller drops that Child.
        child.id()
    }

    pub(super) fn spawn(command: &mut Command) -> Result<Child> {
        let job = win32::create_kill_job()?;
        let mut child = command.creation_flags(win32::HIDDEN_SUSPENDED).spawn()?;
        let setup = || -> Result<Arc<Job>> {
            win32::assign_job(&job, &child)?;
            let root = win32::duplicate_child(&child)?;
            Ok(Arc::new(Job {
                handle: job,
                root,
                monitor: Mutex::new(None),
            }))
        };
        let job = match setup() {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let id = key(&child);
        lock().insert(id, job.clone());
        let monitored = job.clone();
        let monitor = thread::Builder::new()
            .name("dt-child-job".into())
            .spawn(move || {
                // The retained process handle prevents PID reuse from changing what is monitored.
                let _ = win32::wait_process(&monitored.root);
                let _ = win32::terminate_job(&monitored.handle);
                let _ = wait_empty(&monitored.handle);
                let mut jobs = lock();
                if jobs
                    .get(&id)
                    .is_some_and(|current| Arc::ptr_eq(current, &monitored))
                {
                    jobs.remove(&id);
                }
            });
        let monitor = match monitor {
            Ok(monitor) => monitor,
            Err(error) => {
                let _ = win32::terminate_job(&job.handle);
                let _ = child.wait();
                lock().remove(&id);
                return Err(error.into());
            }
        };
        *job.monitor
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(monitor);
        if let Err(error) = win32::resume_child(&child) {
            let _ = win32::terminate_job(&job.handle);
            let _ = child.wait();
            let _ = join_monitor(&job);
            lock().remove(&id);
            return Err(error);
        }
        Ok(child)
    }

    fn wait_empty(job: &OwnedHandle) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while win32::job_active_processes(job)? != 0 {
            if Instant::now() >= deadline {
                return Err(Error::Protocol("process job shutdown timed out"));
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    fn join_monitor(job: &Job) -> Result<()> {
        let monitor = job
            .monitor
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(monitor) = monitor {
            monitor
                .join()
                .map_err(|_| Error::Protocol("process job monitor panicked"))?;
        }
        Ok(())
    }

    pub(super) fn kill(child: &mut Child) -> Result<()> {
        let job = lock().get(&key(child)).cloned();
        if let Some(job) = job {
            win32::terminate_job(&job.handle)?;
            child.wait()?;
            wait_empty(&job.handle)?;
            join_monitor(&job)?;
            lock().remove(&key(child));
        } else if child.try_wait()?.is_none() {
            let status = Command::new("taskkill.exe")
                .args(["/T", "/F", "/PID", &child.id().to_string()])
                .creation_flags(win32::HIDDEN)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?;
            if !status.success() && child.try_wait()?.is_none() {
                return Err(Error::ExternalTool {
                    program: "taskkill",
                    code: status.code(),
                });
            }
            child.wait()?;
        }
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests;
