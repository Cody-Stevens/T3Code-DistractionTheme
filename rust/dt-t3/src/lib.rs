//! T3 API, files, first-run setup and favicon scheduling. All integration operations are stubs.
//! No token, runtime file, settings, theme, or project is accessed by merely constructing a value.

pub mod api;
pub mod favicon;
pub mod setup;
pub mod theme;

use dt_core::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// File locations ported from `t3api.T3_HOME/RUNTIME_FILE/TOKEN_FILE` and `service.SETTINGS`.
#[derive(Clone, Debug)]
pub struct T3Paths {
    /// T3 home, default ~/.t3 or T3CODE_HOME.
    pub home: PathBuf,
    /// userdata/server-runtime.json.
    pub runtime_file: PathBuf,
    /// ~/.secrets/t3code-distraction-theme/t3-token or DISTRACTION_T3_TOKEN_FILE.
    /// This is a path only; no API may print its contents.
    pub token_file: PathBuf,
    /// userdata/themes or T3_THEMES_DIR.
    pub themes_dir: PathBuf,
    /// userdata/settings.json.
    pub settings_file: PathBuf,
    /// ~/.t3/bin/t3.cmd on Windows, t3 elsewhere.
    pub cli: PathBuf,
}

impl T3Paths {
    /// Port Python's path defaults/environment overrides; do not read or create files.
    /// Tests should pass explicit temporary paths instead of discovering the user's environment.
    pub fn discover() -> Result<Self> {
        todo!("resolve T3 paths")
    }
}

/// Server location/liveness from `t3api.origin` and `service.t3_running`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeInfo {
    /// HTTP(S) server origin; validate before attaching an authorization header.
    pub origin: String,
    /// Server process ID.
    pub pid: u32,
}

/// Port `t3api.origin`: read the runtime file on each call so T3 restarts change the destination.
/// Missing/invalid file means T3 is unavailable; no token is read by this function.
pub fn read_runtime(_paths: &T3Paths) -> Result<RuntimeInfo> {
    todo!("read server-runtime.json")
}
