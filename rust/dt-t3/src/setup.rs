//! First-run setup from `service.py`; this module is never called by the working ctl client.

use crate::{
    T3Paths,
    api::{Project, T3Api},
    theme::ThemeDocument,
};
use dt_core::{Error, Result};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

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
pub fn ensure_token(paths: &T3Paths, permissions: &dyn TokenPermissions) -> Result<()> {
    ensure_token_with(paths, permissions, &CliTokenIssuer)
}

/// Token-issuance boundary for tests; returned secret values must never be logged.
pub trait TokenIssuer {
    /// Issue only the two orchestration scopes, with a 90-day TTL and a helper label.
    fn issue(&self, cli: &Path) -> Result<String>;
}

/// Runs the installed CLI without passing a token in its arguments or environment.
pub struct CliTokenIssuer;

impl TokenIssuer for CliTokenIssuer {
    fn issue(&self, cli: &Path) -> Result<String> {
        let mut command = Command::new(cli);
        command
            .args([
                "auth",
                "session",
                "issue",
                "--scope",
                "orchestration:read",
                "--scope",
                "orchestration:operate",
                "--ttl",
                "90d",
                "--label",
                "GlowTube helper",
                "--subject",
                "mcp-client",
                "--json",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or(Error::Protocol("missing token issuer pipe"))?;
        // Drain concurrently so a full pipe cannot deadlock the timeout. Bound retained output.
        let reader = std::thread::spawn(move || {
            let mut output = Vec::new();
            let mut stdout = stdout;
            let mut buffer = [0; 4096];
            loop {
                let count = stdout.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                if output.len() + count <= 65_536 {
                    output.extend_from_slice(&buffer[..count]);
                }
            }
            Ok::<_, std::io::Error>(output)
        });
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if start.elapsed() < Duration::from_secs(60) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                outcome => {
                    // Kill the entire CLI process tree, including a Windows .cmd launcher's child.
                    #[cfg(windows)]
                    let _ = Command::new("taskkill")
                        .args(["/T", "/F", "/PID", &child.id().to_string()])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                    #[cfg(unix)]
                    let _ = Command::new("kill")
                        .args(["-KILL", "--", &format!("-{}", child.id())])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(match outcome {
                        Err(error) => Error::Io(error),
                        _ => Error::Protocol("T3 token issuance timed out"),
                    });
                }
            }
        };
        let output = reader
            .join()
            .map_err(|_| Error::Protocol("token issuer reader failed"))??;
        let status = status?;
        if !status.success() {
            return Err(Error::ExternalTool {
                program: "t3",
                code: status.code(),
            });
        }
        let value: serde_json::Value = serde_json::from_slice(&output)
            .map_err(|_| Error::Protocol("invalid token issuer JSON"))?;
        value
            .get("token")
            .and_then(serde_json::Value::as_str)
            .filter(|token| !token.trim().is_empty())
            .map(str::to_owned)
            .ok_or(Error::Protocol("token issuer returned no token"))
    }
}

/// Injectable setup path. Secure the directory before issuance and the temporary file before writing.
pub fn ensure_token_with(
    paths: &T3Paths,
    permissions: &dyn TokenPermissions,
    issuer: &dyn TokenIssuer,
) -> Result<()> {
    let parent = paths
        .token_file
        .parent()
        .ok_or(Error::InvalidInput("token path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    permissions.restrict_to_owner(parent)?;
    match std::fs::read_to_string(&paths.token_file) {
        Ok(token) if !token.trim().is_empty() => {
            permissions.restrict_to_owner(&paths.token_file)?;
            return Ok(());
        }
        Ok(_) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    let token = issuer.issue(&paths.cli)?;
    let token = token.trim();
    if token.is_empty() || token.contains(['\r', '\n']) {
        return Err(Error::Protocol("token issuer returned an invalid token"));
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    permissions.restrict_to_owner(temp.path())?;
    temp.write_all(token.as_bytes())?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    crate::theme::atomic_persist(temp, &paths.token_file)?;
    permissions.restrict_to_owner(&paths.token_file)
}

/// Port `service.ensure_project`: find/create GlowTube by workspace root/title and pin it.
pub fn ensure_project(api: &T3Api, config: &SetupConfig) -> Result<Project> {
    ensure_deck(api, "GlowTube", &config.workspace_root)
}

fn ensure_deck(api: &T3Api, title: &str, root: &Path) -> Result<Project> {
    std::fs::create_dir_all(root)?;
    let project = match api.find_project(title, Some(root))? {
        Some(project) => project,
        None => api.create_project(title, root)?,
    };
    if project.pinned_at.as_deref().is_none_or(str::is_empty) {
        api.update_project(&project.id, [("pinned".into(), true.into())].into())?;
    }
    Ok(project)
}

/// Port `service.ensure_theme`: atomically write idle theme and select it while preserving settings.
pub fn ensure_theme(config: &SetupConfig) -> Result<()> {
    crate::theme::ThemeWriter {
        themes_dir: config.paths.themes_dir.clone(),
    }
    .write(&config.idle_theme)?;
    crate::theme::select_theme(&config.paths.settings_file, "glowtube")
}

/// Port `service.setup`: ensure token, both pinned projects, and theme; return deck A's project.
/// This explicit opt-in command mutates local T3 files/state; tests use temp paths/mock HTTP only.
pub fn setup(config: &SetupConfig, permissions: &dyn TokenPermissions) -> Result<Project> {
    Ok(setup_decks(config, permissions)?.deck_a)
}

/// Setup result for the binary, including the outstanding sidebar prerequisite.
#[derive(Clone, Debug)]
pub struct SetupReport {
    /// GlowTube, rooted at workspace_root.
    pub deck_a: Project,
    /// GlowTube B, rooted at workspace_root/deck-b.
    pub deck_b: Project,
    /// True on older servers or when reusing a preexisting token without MCP client identity.
    pub idle_threads_required: bool,
    /// Idle thread IDs when MCP setup succeeds, ordered deck A then B.
    pub idle_thread_ids: Option<[String; 2]>,
}

/// Set up both decks and idle sidebar threads without starting a turn.
/// Older/non-MCP bearer sessions retain project/theme setup and report the thread prerequisite.
pub fn setup_decks(
    config: &SetupConfig,
    permissions: &dyn TokenPermissions,
) -> Result<SetupReport> {
    ensure_token(&config.paths, permissions)?;
    let api = T3Api::new(config.paths.clone())?;
    let deck_a = ensure_project(&api, config)?;
    let deck_b = ensure_deck(&api, "GlowTube B", &config.workspace_root.join("deck-b"))?;
    if deck_a.id == deck_b.id {
        return Err(Error::Protocol("favicon decks must be distinct projects"));
    }
    ensure_theme(config)?;
    let idle_thread_ids = match (|| {
        Ok([
            api.ensure_idle_thread(&deck_a.id)?,
            api.ensure_idle_thread(&deck_b.id)?,
        ])
    })() {
        Ok(ids) => Some(ids),
        Err(Error::Http {
            status: 401 | 403 | 404 | 405,
        }) => None,
        Err(error) => return Err(error),
    };
    Ok(SetupReport {
        deck_a,
        deck_b,
        idle_threads_required: idle_thread_ids.is_none(),
        idle_thread_ids,
    })
}
