//! All API and file tests use temporary paths and an ephemeral mock; no user state is discovered.
mod support;

use dt_core::Error;
use dt_t3::{
    api::{Method, T3Api},
    read_runtime,
    theme::{MAX_CSS_CHARS, ThemeDocument, ThemeWriter, select_theme},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Duration};
use support::*;

#[test]
fn runtime_validates_origin_and_reads_each_request() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    assert!(read_runtime(&paths).is_err());
    for origin in [
        "file:///private",
        "http://user:password@localhost",
        "http://localhost/api",
        "http://localhost/?x=1",
        "http://localhost/#fragment",
        "nonsense",
    ] {
        runtime(&paths, origin);
        assert!(read_runtime(&paths).is_err());
    }
    let first = MockServer::new(|_| (200, json!({"server": 1}).to_string()));
    let second = MockServer::new(|_| (200, json!({"server": 2}).to_string()));
    fake_token(&paths);
    runtime(&paths, &first.origin);
    let api = T3Api::new(paths.clone()).unwrap();
    assert_eq!(
        api.call(Method::Get, "/api/projects", None, Duration::from_secs(1))
            .unwrap()["server"],
        1
    );
    runtime(&paths, &second.origin);
    assert_eq!(
        api.call(Method::Get, "/api/projects", None, Duration::from_secs(1))
            .unwrap()["server"],
        2
    );
    assert_eq!(first.requests.lock().unwrap().len(), 1);
    assert_eq!(second.requests.lock().unwrap().len(), 1);
}

#[test]
fn projects_filter_deleted_preserve_fields_and_prefer_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    let server = MockServer::new(|request| {
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/projects");
        assert!(
            request
                .headers
                .to_ascii_lowercase()
                .contains("authorization: bearer fake-test-bearer\r\n")
        );
        (200, json!({"projects":[
            {"id":"deleted", "title":"GlowTube", "workspaceRoot":"unused", "deletedAt":"yesterday"},
            {"id":"title", "title":"GlowTube", "workspaceRoot":"another"},
            {"id":"root", "title":"Old title", "workspaceRoot":"C:\\Deck", "future":{"kept":true}}
        ]}).to_string())
    });
    fake_token(&paths);
    runtime(&paths, &server.origin);
    let api = T3Api::new(paths).unwrap();
    let projects = api.projects().unwrap();
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[1].extra["future"]["kept"], true);
    let root = if cfg!(windows) { "c:/dEcK" } else { "C:\\Deck" };
    assert_eq!(
        api.find_project("GlowTube", Some(Path::new(root)))
            .unwrap()
            .unwrap()
            .id,
        "root"
    );
    assert_eq!(
        api.find_project("GlowTube", Some(Path::new("missing")))
            .unwrap()
            .unwrap()
            .id,
        "title"
    );
    assert!(api.find_project("Absent", None).unwrap().is_none());
}

#[test]
fn create_update_delete_wire_format_and_empty_response() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    let server = MockServer::new(|request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/projects/mutate");
        let command = request.body["commandId"]
            .as_str()
            .unwrap()
            .strip_prefix("glowtube:")
            .unwrap();
        uuid::Uuid::parse_str(command).unwrap();
        match request.body["type"].as_str().unwrap() {
            "project.create" => {
                uuid::Uuid::parse_str(request.body["projectId"].as_str().unwrap()).unwrap();
                assert_eq!(request.body["title"], "GlowTube");
                (
                    200,
                    json!({"id":"created", "title":"GlowTube", "workspaceRoot":"deck"}).to_string(),
                )
            }
            "project.update" => {
                assert_eq!(request.body["projectId"], "created");
                assert_eq!(request.body["pinned"], true);
                assert_eq!(request.body["faviconPath"], "clip.svg");
                (204, String::new())
            }
            "project.delete" => (200, "null".into()),
            _ => panic!("unexpected mutation"),
        }
    });
    fake_token(&paths);
    runtime(&paths, &server.origin);
    let api = T3Api::new(paths).unwrap();
    let project = api.create_project("GlowTube", Path::new("deck")).unwrap();
    let fields = [
        ("pinned".into(), true.into()),
        ("faviconPath".into(), "clip.svg".into()),
    ]
    .into();
    assert!(api.update_project(&project.id, fields).unwrap().is_null());
    api.mutate(json!({"type":"project.delete", "projectId":project.id, "commandId":"overridden"}))
        .unwrap();
    let requests = server.requests.lock().unwrap();
    let commands: std::collections::HashSet<_> =
        requests.iter().map(|r| &r.body["commandId"]).collect();
    assert_eq!(commands.len(), 3);
}

#[test]
fn errors_are_redacted_and_invalid_paths_never_send_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    fake_token(&paths);
    let server = MockServer::new(|_| (401, "private fake-test-bearer response".into()));
    runtime(&paths, &server.origin);
    let api = T3Api::new(paths.clone()).unwrap();
    let error = api.projects().unwrap_err();
    assert!(matches!(error, Error::Http { status: 401 }));
    assert!(!format!("{error:?} {error}").contains("fake-test"));
    for path in [
        "https://evil/api/projects",
        "//evil/api/projects",
        "/api/../secret",
        "/api/%2e%2e/secret",
        "/api/evil?x=1",
        "/api/evil\\secret",
    ] {
        assert!(
            api.call(Method::Get, path, None, Duration::from_secs(1))
                .is_err()
        );
    }
    assert!(
        api.update_project("id", BTreeMap::from([("type".into(), json!("other"))]))
            .is_err()
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    std::fs::write(&paths.token_file, " \n ").unwrap();
    assert!(T3Api::new(paths.clone()).is_err());
    std::fs::write(&paths.token_file, "token\r\nInjected: header").unwrap();
    assert!(T3Api::new(paths).is_err());
}

#[test]
fn malformed_json_and_redirects_are_errors_without_bodies() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    fake_token(&paths);
    for (status, body) in [
        (200, "fake-test-bearer invalid JSON"),
        (302, "private redirect"),
    ] {
        let server = MockServer::new(move |_| (status, body.into()));
        runtime(&paths, &server.origin);
        let error = T3Api::new(paths.clone()).unwrap().projects().unwrap_err();
        assert!(!format!("{error:?}").contains("fake-test"));
        if status == 302 {
            assert!(matches!(error, Error::Http { status: 302 }));
        }
    }
}

#[test]
fn redirects_do_not_forward_the_bearer_and_request_timeouts_are_honored() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    fake_token(&paths);
    let destination = MockServer::new(|_| (200, "null".into()));
    let target = destination.origin.clone();
    let redirect = MockServer::with_headers(move |_| {
        (
            302,
            vec![("Location".into(), format!("{target}/api/projects"))],
            String::new(),
        )
    });
    runtime(&paths, &redirect.origin);
    assert!(matches!(
        T3Api::new(paths.clone()).unwrap().projects(),
        Err(Error::Http { status: 302 })
    ));
    assert!(destination.requests.lock().unwrap().is_empty());
    let slow = MockServer::new(|_| {
        std::thread::sleep(Duration::from_millis(150));
        (200, "null".into())
    });
    runtime(&paths, &slow.origin);
    let result = T3Api::new(paths).unwrap().call(
        Method::Get,
        "/api/projects",
        None,
        Duration::from_millis(25),
    );
    assert!(matches!(result, Err(Error::Protocol(_))));
}

#[test]
fn atomic_theme_replaces_complete_documents_and_enforces_utf16_budgets() {
    let temp = tempfile::tempdir().unwrap();
    let writer = ThemeWriter {
        themes_dir: temp.path().join("themes"),
    };
    let document = ThemeDocument::glowtube("x".repeat(MAX_CSS_CHARS)).unwrap();
    writer.write(&document).unwrap();
    let replacement = ThemeDocument::glowtube("body{color:red}".into()).unwrap();
    writer.write(&replacement).unwrap();
    let path = writer.themes_dir.join("glowtube.json");
    let current: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(current["css"], replacement.css);
    assert_eq!(std::fs::read_dir(&writer.themes_dir).unwrap().count(), 1);
    assert!(ThemeDocument::glowtube("x".repeat(MAX_CSS_CHARS + 1)).is_err());
    assert!(ThemeDocument::glowtube("😀".repeat(MAX_CSS_CHARS / 2 + 1)).is_err());
    let mut oversized = replacement.clone();
    oversized.name = "😀".repeat(25);
    assert!(writer.write(&oversized).is_err());
    oversized.name = "a".repeat(48);
    assert!(writer.write(&oversized).is_ok());
}

#[test]
fn settings_preserve_unknown_values_and_refuse_corrupt_input() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let original =
        json!({"nested":{"array":[1,null,"retained"]}, "future":true, "defaultTheme":"old"});
    std::fs::write(&path, original.to_string()).unwrap();
    select_theme(&path, "glowtube").unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["nested"], original["nested"]);
    assert_eq!(value["future"], true);
    assert_eq!(value["defaultAppearance"], "dark");
    assert_eq!(value["defaultTheme"], "glowtube");
    assert_eq!(value["defaultThemeSetAt"], value["defaultAppearanceSetAt"]);
    assert!(
        value["defaultThemeSetAt"]
            .as_str()
            .unwrap()
            .ends_with(".000Z")
    );
    select_theme(&path, "glowtube").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    for bad in ["{broken", "[]"] {
        std::fs::write(&path, bad).unwrap();
        assert!(select_theme(&path, "glowtube").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
    }
    let missing = temp.path().join("new/settings.json");
    select_theme(&missing, "glowtube").unwrap();
    assert!(missing.is_file());
}

#[test]
fn theme_replacement_never_exposes_partial_json_and_measures_write_overhead() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let temp = tempfile::tempdir().unwrap();
    let writer = ThemeWriter {
        themes_dir: temp.path().into(),
    };
    let document = ThemeDocument::glowtube("x".repeat(MAX_CSS_CHARS)).unwrap();
    writer.write(&document).unwrap();
    let path = temp.path().join("glowtube.json");
    let done = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let (stopping, count) = (done.clone(), reads.clone());
    let reader = std::thread::spawn(move || {
        while !stopping.load(Ordering::SeqCst) {
            let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(value["css"].as_str().unwrap().len(), MAX_CSS_CHARS);
            count.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut times = Vec::new();
    for _ in 0..40 {
        let start = std::time::Instant::now();
        writer.write(&document).unwrap();
        times.push(start.elapsed());
    }
    done.store(true, Ordering::SeqCst);
    reader.join().unwrap();
    times.sort();
    assert!(reads.load(Ordering::SeqCst) > 0);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    println!(
        "49,500-character atomic write overhead over 40 writes: median {:?}, worst {:?}",
        times[20], times[39]
    );
}

#[cfg(windows)]
#[test]
fn locked_theme_reports_exhausted_retries_and_cleans_up_its_temporary_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = tempfile::tempdir().unwrap();
    let writer = ThemeWriter {
        themes_dir: temp.path().into(),
    };
    writer
        .write(&ThemeDocument::glowtube("old".into()).unwrap())
        .unwrap();
    let path = temp.path().join("glowtube.json");
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let replacement = ThemeDocument::glowtube("new".into()).unwrap();
    assert!(writer.write(&replacement).is_err());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    assert!(std::fs::read_to_string(&path).unwrap().contains("old"));
    drop(held);
    writer.write(&replacement).unwrap();
    assert!(std::fs::read_to_string(path).unwrap().contains("new"));
}
