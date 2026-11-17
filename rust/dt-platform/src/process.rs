//! Lifecycle and ACL primitives from `service.pid_alive/single_instance/start_player/stop_player`.

use dt_core::Result;
use std::{
    path::Path,
    process::{Child, Command},
};

/// Held named mutex from `service.single_instance`; keep it alive throughout supervisor execution.
/// Implementation closes the handle on drop, without releasing another process's mutex.
pub struct InstanceGuard {
    _private: (),
}

/// Port `service.single_instance`: CreateMutexW with `Local\GlowTubeService`; return None when
/// already held. The caller supplies a name so tests can use unique isolated mutexes.
pub fn single_instance(_name: &str) -> Result<Option<InstanceGuard>> {
    todo!("single-instance mutex")
}

/// Port `service.pid_alive`: OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION), GetExitCodeProcess
/// == STILL_ACTIVE (259), close handle. PID zero or inaccessible/missing process returns false.
pub fn pid_alive(_pid: u32) -> bool {
    todo!("Win32 process liveness")
}

/// Port `service.start_player`: CREATE_NO_WINDOW, redirected logs, return an owned child.
/// Command arguments are already separated; never build a shell command containing user URLs.
pub fn spawn_hidden(_command: &mut Command) -> Result<Child> {
    todo!("spawn hidden child")
}

/// Port `service.stop_player` fallback: kill the entire OWNED process tree (Job Object or
/// taskkill /T /F /PID), wait/reap it, and ensure descendants stop. Never kill just the parent shell.
/// The supervisor first requests /quit and waits up to 10 seconds; this is its fallback only.
pub fn kill_process_tree(_child: &mut Child) -> Result<()> {
    todo!("kill and reap owned process tree")
}

/// Port `service.ensure_token`'s icacls operations: remove inheritance and grant only the current
/// owner access to a token file/directory. Return errors instead of leaving a readable secret.
/// This API takes a path, never a token value. Setup invokes it before publishing secret contents.
pub fn restrict_to_owner(_path: &Path) -> Result<()> {
    todo!("owner-only token ACL")
}
