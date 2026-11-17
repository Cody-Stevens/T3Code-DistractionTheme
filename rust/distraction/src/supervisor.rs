//! Process supervision from `service.py`; first-run T3 setup remains in dt-t3.

use dt_core::{Error, Result};
use dt_t3::T3Paths;
use std::{path::Path, process::Child};

/// Port `service.t3_running`: read runtime PID, delegate liveness to dt-platform; invalid/missing
/// runtime means false. Poll at 3 s; never infer liveness solely from the runtime file existing.
pub fn t3_running(_paths: &T3Paths) -> bool {
    todo!("check T3 server liveness")
}

/// Port `service.start_player`: run this executable with play, hidden, with append-only local logs.
/// Own/reap the returned child; no shell and no arbitrary command-line strings.
pub fn start_player(_executable: &Path, _state_dir: &Path) -> Result<Child> {
    todo!("spawn supervised player")
}

/// Port `service.stop_player`: /quit with 3 s timeout, wait 10 s, then kill whole owned process tree.
/// Never use this during development against the existing Python helper.
pub fn stop_player(_child: &mut Child, _port: u16) -> Result<()> {
    todo!("stop owned player")
}

/// Port `service.serve`: hold Local\GlowTubeService mutex, setup/start while T3 runs, stop on exit,
/// restart after crashes at min(60,2^crashes) seconds, reset after 300 calm seconds; setup retry 30 s.
/// Phase one returns before creating a mutex, spawning a child or mutating T3 state.
pub fn serve() -> Result<()> {
    Err(Error::NotImplemented("service"))
}
