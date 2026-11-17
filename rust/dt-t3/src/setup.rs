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

/// Port `service.ensure_token`: reuse nonempty token or issue via t3 auth session issue (generic
/// subject), two orchestration scopes only, TTL 90d, label GlowTube helper, timeout 60 s. Apply owner-only ACL
/// to directory/file before exposing the value. Never echo the command's stdout or secret value.
pub fn ensure_token(paths: &T3Paths, permissions: &dyn TokenPermissions) -> Result<()> {
    ensure_token_with(paths, permissions, &CliTokenIssuer)
}

/// A short-lived session for T3's `/mcp` endpoint. The token stays in memory; it is never written,
/// logged or displayed, and this type implements neither Debug nor Serialize.
pub struct McpSessionToken {
    /// T3's session id, used to revoke the session after setup.
    pub session_id: String,
    token: String,
}

impl McpSessionToken {
    /// Wrap an issued session (tests construct fakes with this).
    pub fn new(session_id: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            token: token.into(),
        }
    }

    /// The bearer value, for building the setup-only client.
    pub fn token(&self) -> &str {
        &self.token
    }
}

/// Token-issuance boundary for tests; returned secret values must never be logged.
pub trait TokenIssuer {
    /// Issue the helper's long-lived token: only the two orchestration scopes, a 90-day TTL and a
    /// helper label. It is a generic session because T3 accepts `mcp-client` sessions only on
    /// `/mcp`, and the helper uses the HTTP API.
    fn issue(&self, cli: &Path) -> Result<String>;

    /// Issue a 15-minute `mcp-client` session with the same two scopes. Setup uses it only to
    /// create the idle sidebar threads and revokes it right after.
    fn issue_mcp(&self, _cli: &Path) -> Result<McpSessionToken> {
        Err(Error::Protocol("MCP session issuance unsupported"))
    }

    /// Revoke a session issued by [`TokenIssuer::issue_mcp`].
    fn revoke(&self, _cli: &Path, _session_id: &str) -> Result<()> {
        Ok(())
    }
}

/// Arguments for the helper's long-lived token.
pub const HELPER_TOKEN_ARGS: [&str; 12] = [
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
    "--json",
];

/// Arguments for setup's short-lived MCP session.
pub const SETUP_MCP_ARGS: [&str; 14] = [
    "auth",
    "session",
    "issue",
    "--scope",
    "orchestration:read",
    "--scope",
    "orchestration:operate",
    "--ttl",
    "15m",
    "--label",
    "GlowTube setup",
    "--subject",
    "mcp-client",
    "--json",
];

/// Runs the installed CLI without passing a token in its arguments or environment.
pub struct CliTokenIssuer;

impl TokenIssuer for CliTokenIssuer {
    fn issue(&self, cli: &Path) -> Result<String> {
        let value = issued_json(&run_cli(cli, &HELPER_TOKEN_ARGS)?)?;
        issued_field(&value, "token")
    }

    fn issue_mcp(&self, cli: &Path) -> Result<McpSessionToken> {
        let value = issued_json(&run_cli(cli, &SETUP_MCP_ARGS)?)?;
        let session_id = issued_field(&value, "sessionId")?;
        if !valid_session_id(&session_id) {
            return Err(Error::Protocol(
                "token issuer returned an invalid session id",
            ));
        }
        Ok(McpSessionToken::new(
            session_id,
            issued_field(&value, "token")?,
        ))
    }

    fn revoke(&self, cli: &Path, session_id: &str) -> Result<()> {
        if !valid_session_id(session_id) {
            return Err(Error::InvalidInput("invalid session id"));
        }
        run_cli(cli, &["auth", "session", "revoke", session_id]).map(drop)
    }
}

/// Session ids go on the CLI's command line, so accept only plain id characters (no leading `-`).
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn issued_json(output: &[u8]) -> Result<serde_json::Value> {
    serde_json::from_slice(output).map_err(|_| Error::Protocol("invalid token issuer JSON"))
}

fn issued_field(value: &serde_json::Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
        .ok_or(Error::Protocol("token issuer returned no token"))
}

/// Run the T3 CLI with a 60 s limit, killing its whole tree on timeout. Returns bounded stdout,
/// which callers must never log (it can hold a token).
fn run_cli(cli: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new(cli);
    command
        .args(args)
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
    Ok(output)
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
    setup_decks_with(config, permissions, &CliTokenIssuer)
}

/// [`setup_decks`] with an injected issuer. The helper token is reused or issued as a generic
/// session; idle threads are created with a short-lived `mcp-client` session that is revoked
/// before returning, whether or not thread creation succeeded.
pub fn setup_decks_with(
    config: &SetupConfig,
    permissions: &dyn TokenPermissions,
    issuer: &dyn TokenIssuer,
) -> Result<SetupReport> {
    ensure_token_with(&config.paths, permissions, issuer)?;
    let api = T3Api::new(config.paths.clone())?;
    let deck_a = ensure_project(&api, config)?;
    let deck_b = ensure_deck(&api, "GlowTube B", &config.workspace_root.join("deck-b"))?;
    if deck_a.id == deck_b.id {
        return Err(Error::Protocol("favicon decks must be distinct projects"));
    }
    ensure_theme(config)?;
    let idle_thread_ids = match issuer.issue_mcp(&config.paths.cli) {
        // No MCP session (older T3, CLI missing): projects and theme are done; report the threads.
        Err(_) => None,
        Ok(session) => {
            let created =
                T3Api::with_token(config.paths.clone(), session.token()).and_then(|mcp| {
                    Ok([
                        mcp.ensure_idle_thread(&deck_a.id)?,
                        mcp.ensure_idle_thread(&deck_b.id)?,
                    ])
                });
            let _ = issuer.revoke(&config.paths.cli, &session.session_id);
            match created {
                Ok(ids) => Some(ids),
                Err(Error::Http {
                    status: 401 | 403 | 404 | 405,
                }) => None,
                Err(error) => return Err(error),
            }
        }
    };
    Ok(SetupReport {
        deck_a,
        deck_b,
        idle_threads_required: idle_thread_ids.is_none(),
        idle_thread_ids,
    })
}
