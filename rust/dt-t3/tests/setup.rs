//! Setup runs only against temporary paths, injected issuance/permissions and an HTTP mock.
mod support;

use dt_core::{Error, Result};
use dt_t3::{
    setup::{
        McpSessionToken, SetupConfig, TokenIssuer, TokenPermissions, ensure_token_with,
        setup_decks_with,
    },
    theme::ThemeDocument,
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use support::*;

#[derive(Default)]
struct Permissions {
    calls: RefCell<Vec<PathBuf>>,
    fail: Cell<bool>,
}
impl TokenPermissions for Permissions {
    fn restrict_to_owner(&self, path: &Path) -> Result<()> {
        self.calls.borrow_mut().push(path.into());
        if self.fail.get() {
            return Err(Error::Protocol("mock ACL failure"));
        }
        // A temp file must still be empty when its ACL is applied, before the secret is written.
        if path.is_file() && path.file_name().unwrap() != "t3-token" {
            assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
        }
        Ok(())
    }
}

#[derive(Default)]
struct Issuer {
    calls: Cell<usize>,
}
impl TokenIssuer for Issuer {
    fn issue(&self, _: &Path) -> Result<String> {
        self.calls.set(self.calls.get() + 1);
        Ok("fake-issued-token".into())
    }
}

/// Issues a fake setup MCP session and records revocations.
#[derive(Default)]
struct McpIssuer {
    issued: Cell<usize>,
    revoked: RefCell<Vec<String>>,
}
impl TokenIssuer for McpIssuer {
    fn issue(&self, _: &Path) -> Result<String> {
        panic!("the helper token already exists")
    }
    fn issue_mcp(&self, _: &Path) -> Result<McpSessionToken> {
        self.issued.set(self.issued.get() + 1);
        Ok(McpSessionToken::new(
            format!("setup-session-{}", self.issued.get()),
            "fake-mcp-bearer",
        ))
    }
    fn revoke(&self, _: &Path, session_id: &str) -> Result<()> {
        self.revoked.borrow_mut().push(session_id.into());
        Ok(())
    }
}

#[test]
fn token_is_protected_before_write_and_reused_on_second_setup() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    let permissions = Permissions::default();
    let issuer = Issuer::default();
    ensure_token_with(&paths, &permissions, &issuer).unwrap();
    assert_eq!(
        std::fs::read_to_string(&paths.token_file).unwrap(),
        "fake-issued-token"
    );
    assert_eq!(issuer.calls.get(), 1);
    let calls = permissions.calls.borrow();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0], paths.token_file.parent().unwrap());
    assert_eq!(calls[2], paths.token_file);
    drop(calls);
    ensure_token_with(&paths, &permissions, &issuer).unwrap();
    assert_eq!(issuer.calls.get(), 1);
}

#[test]
fn acl_failure_prevents_issuance_or_exposure() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    let permissions = Permissions::default();
    permissions.fail.set(true);
    let issuer = Issuer::default();
    assert!(ensure_token_with(&paths, &permissions, &issuer).is_err());
    assert_eq!(issuer.calls.get(), 0);
    assert!(!paths.token_file.exists());
}

#[test]
fn empty_token_is_reissued_and_invalid_tool_output_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    fake_token(&paths);
    std::fs::write(&paths.token_file, " \n").unwrap();
    let issuer = Issuer::default();
    ensure_token_with(&paths, &Permissions::default(), &issuer).unwrap();
    assert_eq!(issuer.calls.get(), 1);
    struct InvalidIssuer;
    impl TokenIssuer for InvalidIssuer {
        fn issue(&self, _: &Path) -> Result<String> {
            Ok("invalid\nheader".into())
        }
    }
    std::fs::write(&paths.token_file, "").unwrap();
    assert!(ensure_token_with(&paths, &Permissions::default(), &InvalidIssuer).is_err());
    assert_eq!(std::fs::metadata(paths.token_file).unwrap().len(), 0);
}

#[test]
fn setup_creates_pins_both_decks_and_is_idempotent_without_starting_turns() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    fake_token(&paths);
    let projects = Arc::new(Mutex::new(Vec::<Value>::new()));
    let saved = projects.clone();
    let threads = Arc::new(Mutex::new(
        std::collections::BTreeMap::<String, String>::new(),
    ));
    let saved_threads = threads.clone();
    let server = MockServer::new(move |request| {
        let mut projects = saved.lock().unwrap();
        let bearer = if request.path == "/mcp" {
            "authorization: bearer fake-mcp-bearer"
        } else {
            "authorization: bearer fake-test-bearer"
        };
        assert!(request.headers.to_ascii_lowercase().contains(bearer));
        match request.path.as_str() {
            "/api/projects" => (200, json!({"projects": *projects}).to_string()),
            "/api/projects/mutate" => match request.body["type"].as_str().unwrap() {
                "project.create" => {
                    let project = json!({"id": request.body["projectId"], "title": request.body["title"], "workspaceRoot": request.body["workspaceRoot"]});
                    projects.push(project.clone());
                    (200, project.to_string())
                }
                "project.update" => {
                    assert_eq!(request.body["pinned"], true);
                    let project = projects
                        .iter_mut()
                        .find(|p| p["id"] == request.body["projectId"])
                        .unwrap();
                    project["pinnedAt"] = "today".into();
                    (200, project.to_string())
                }
                _ => panic!("unexpected mutation"),
            },
            "/mcp" => match request.body["method"].as_str() {
                Some("initialize") => (200, json!({"jsonrpc":"2.0", "id":request.body["id"], "result":{"protocolVersion":"2025-06-18"}}).to_string()),
                Some("notifications/initialized") => (202, String::new()),
                Some("tools/call") => {
                    let args = &request.body["params"]["arguments"];
                    let project_id = args["projectId"].as_str().unwrap();
                    let mut threads = saved_threads.lock().unwrap();
                    let result = match request.body["params"]["name"].as_str().unwrap() {
                        "t3_thread_list" => json!({"threads": threads.get(project_id).map(|id| vec![json!({"threadId":id, "projectId":project_id, "status":"idle"})]).unwrap_or_default()}),
                        "t3_thread_launch" => {
                            assert!(args.get("message").is_none());
                            assert!(args.get("attachments").is_none());
                            assert_eq!(args["workspaceStrategy"]["type"], "root");
                            let thread_id = format!("idle-{project_id}");
                            threads.insert(project_id.into(), thread_id.clone());
                            json!({"threadId":thread_id, "projectId":project_id, "runId":null})
                        }
                        _ => panic!("setup must never dispatch a message"),
                    };
                    (200, json!({"jsonrpc":"2.0", "id":request.body["id"], "result":{"structuredContent":result}}).to_string())
                }
                _ => panic!("unexpected MCP method"),
            },
            _ => panic!("unexpected setup endpoint"),
        }
    });
    runtime(&paths, &server.origin);
    let config = SetupConfig {
        paths: paths.clone(),
        workspace_root: temp.path().join("workspace"),
        idle_theme: ThemeDocument::glowtube("body{}".into()).unwrap(),
    };
    let mcp = McpIssuer::default();
    let first = setup_decks_with(&config, &Permissions::default(), &mcp).unwrap();
    assert_ne!(first.deck_a.id, first.deck_b.id);
    assert_eq!(first.deck_a.title, "GlowTube");
    assert_eq!(first.deck_b.title, "GlowTube B");
    assert_eq!(
        Path::new(&first.deck_b.workspace_root),
        config.workspace_root.join("deck-b")
    );
    assert!(!first.idle_threads_required);
    assert_eq!(first.idle_thread_ids.as_ref().unwrap().len(), 2);
    assert!(paths.themes_dir.join("glowtube.json").is_file());
    let second = setup_decks_with(&config, &Permissions::default(), &mcp).unwrap();
    // One short-lived MCP session per setup, each revoked afterwards.
    assert_eq!(mcp.issued.get(), 2);
    assert_eq!(
        *mcp.revoked.borrow(),
        ["setup-session-1", "setup-session-2"]
    );
    assert_eq!(first.deck_a.id, second.deck_a.id);
    assert_eq!(first.deck_b.id, second.deck_b.id);
    let requests = server.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method == "POST" && r.path == "/api/projects/mutate")
            .count(),
        4
    );
    assert_eq!(projects.lock().unwrap().len(), 2);
    assert_eq!(threads.lock().unwrap().len(), 2);
}

#[test]
fn installed_cli_adapter_passes_only_the_required_scopes() {
    use dt_t3::setup::CliTokenIssuer;
    let temp = tempfile::tempdir().unwrap();
    let cli = temp.path().join(if cfg!(windows) {
        "fake-t3.cmd"
    } else {
        "fake-t3"
    });
    let script = if cfg!(windows) {
        let checks: String = [
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
        ]
        .into_iter()
        .map(|argument| format!("if not \"%~1\"==\"{argument}\" exit /b 9\r\nshift\r\n"))
        .collect();
        format!("@echo off\r\n{checks}echo {{\"token\":\"fake-command-token\"}}\r\n")
    } else {
        "#!/bin/sh\n[ \"$*\" = 'auth session issue --scope orchestration:read --scope orchestration:operate --ttl 90d --label GlowTube helper --json' ] || exit 9\nprintf '%s' '{\"token\":\"fake-command-token\"}'\n".into()
    };
    std::fs::write(&cli, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert_eq!(CliTokenIssuer.issue(&cli).unwrap(), "fake-command-token");
}

#[test]
fn setup_without_an_mcp_session_still_sets_up_projects_and_reports_threads() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    fake_token(&paths);
    let projects = Arc::new(Mutex::new(Vec::<Value>::new()));
    let saved = projects.clone();
    let server = MockServer::new(move |request| {
        let mut projects = saved.lock().unwrap();
        match request.path.as_str() {
            "/api/projects" => (200, json!({"projects": *projects}).to_string()),
            "/api/projects/mutate" => {
                let project = json!({"id": request.body["projectId"], "title": request.body["title"],
                    "workspaceRoot": request.body["workspaceRoot"], "pinnedAt": "today"});
                if request.body["type"] == "project.create" {
                    projects.push(project.clone());
                }
                (200, project.to_string())
            }
            _ => panic!("no MCP call without a setup session"),
        }
    });
    runtime(&paths, &server.origin);
    let config = SetupConfig {
        paths: paths.clone(),
        workspace_root: temp.path().join("workspace"),
        idle_theme: ThemeDocument::glowtube("body{}".into()).unwrap(),
    };
    // The default issue_mcp reports no MCP support.
    let report = setup_decks_with(&config, &Permissions::default(), &Issuer::default()).unwrap();
    assert!(report.idle_threads_required);
    assert!(report.idle_thread_ids.is_none());
}

#[test]
fn session_ids_are_plain_before_reaching_the_command_line() {
    use dt_t3::setup::valid_session_id;
    assert!(valid_session_id("01HZX9-session_id"));
    assert!(!valid_session_id(""));
    assert!(!valid_session_id("--json"));
    assert!(!valid_session_id("id with space"));
    assert!(!valid_session_id("id;rm"));
}
