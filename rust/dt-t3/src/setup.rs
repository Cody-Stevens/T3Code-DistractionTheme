//! First-run setup from `service.py`; this module is never called by the working ctl client.

use crate::{
    T3Paths,
    api::{Project, T3Api},
    theme::ThemeDocument,
};
use dt_core::Result;
use std::path::{Path, PathBuf};

/// Explicit setup inputs; no setup step runs as a side effect of CLI argument parsing.
#[derive(Clone, Debug)]
pub struct SetupConfig {
    /// All user file paths, injectable for tests.
    pub paths: T3Paths,
    /// GlowTube project workspace root.
    pub workspace_root: PathBuf,
    /// Rendered idle theme supplied by the binary (dt-t3 never depends on dt-render).
    pub idle_theme: ThemeDocument,
}

/// Platform adapter for `service.ensure_token`'s owner-only ACL requirement.
/// The binary implements this through dt-platform, preserving downward-only crate dependencies.
pub trait TokenPermissions {
    /// Restrict a file or directory to the current owner; never read/return a token value.
    fn restrict_to_owner(&self, path: &Path) -> Result<()>;
}

/// Port `service.ensure_token`: reuse nonempty token or issue via t3 auth session issue, two
/// orchestration scopes only, TTL 90d, label GlowTube helper, timeout 60 s. Apply owner-only ACL
/// to directory/file before exposing the value. Never echo the command's stdout or secret value.
pub fn ensure_token(_paths: &T3Paths, _permissions: &dyn TokenPermissions) -> Result<()> {
    todo!("ensure scoped owner-only token")
}

/// Port `service.ensure_project`: find/create GlowTube by workspace root/title and pin it.
pub fn ensure_project(_api: &T3Api, _config: &SetupConfig) -> Result<Project> {
    todo!("ensure pinned project")
}

/// Port `service.ensure_theme`: atomically write idle theme and select it while preserving settings.
pub fn ensure_theme(_config: &SetupConfig) -> Result<()> {
    todo!("ensure selected theme")
}

/// Port `service.setup`: ensure token, project, and theme in that order; return the project.
/// This explicit opt-in command mutates local T3 files/state; tests use temp paths/mock HTTP only.
pub fn setup(_config: &SetupConfig, _permissions: &dyn TokenPermissions) -> Result<Project> {
    todo!("first-run setup")
}
