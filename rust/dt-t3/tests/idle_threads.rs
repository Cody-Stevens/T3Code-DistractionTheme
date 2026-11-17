//! Source-verified MCP transport, exercised entirely against an ephemeral HTTP mock.
mod support;

use dt_core::Error;
use dt_t3::api::T3Api;
use serde_json::json;
use support::*;

#[test]
fn sse_handshake_preserves_session_and_creates_idle_without_a_message() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    fake_token(&paths);
    let server = MockServer::with_headers(|request| {
        assert_eq!(request.path, "/mcp");
        assert!(
            request
                .headers
                .to_ascii_lowercase()
                .contains("authorization: bearer fake-test-bearer")
        );
        if request.method == "DELETE" {
            assert!(request.headers.contains("fake-mcp-session"));
            return (204, Vec::new(), String::new());
        }
        let result = match request.body["method"].as_str().unwrap() {
            "initialize" => {
                assert_eq!(request.body["params"]["protocolVersion"], "2025-06-18");
                json!({"protocolVersion":"2025-06-18"})
            }
            "notifications/initialized" => return (202, Vec::new(), String::new()),
            "tools/call" => {
                assert!(request.headers.contains("fake-mcp-session"));
                assert!(
                    request
                        .headers
                        .to_ascii_lowercase()
                        .contains("mcp-protocol-version: 2025-06-18")
                );
                let args = &request.body["params"]["arguments"];
                assert_eq!(args["projectId"], "deck-a");
                match request.body["params"]["name"].as_str().unwrap() {
                    "t3_thread_list" => {
                        assert_eq!(args["statuses"], json!(["idle"]));
                        assert_eq!(args["settled"], false);
                        assert_eq!(args["snoozed"], false);
                        json!({"structuredContent":{"threads":[]}})
                    }
                    "t3_thread_launch" => {
                        assert!(args.get("message").is_none());
                        assert!(args.get("attachments").is_none());
                        assert_eq!(args["workspaceStrategy"], json!({"type":"root"}));
                        assert!(
                            args["clientRequestId"]
                                .as_str()
                                .unwrap()
                                .starts_with("glowtube-idle:")
                        );
                        json!({"structuredContent":{"threadId":"idle-a", "projectId":"deck-a", "runId":null}})
                    }
                    _ => panic!("unexpected tool"),
                }
            }
            _ => panic!("unexpected MCP method"),
        };
        let envelope = json!({"jsonrpc":"2.0", "id":request.body["id"], "result":result});
        (
            200,
            vec![
                ("Content-Type".into(), "text/event-stream".into()),
                ("Mcp-Session-Id".into(), "fake-mcp-session".into()),
            ],
            format!(": heartbeat\n\nevent: message\ndata: {envelope}\n\n"),
        )
    });
    runtime(&paths, &server.origin);
    assert_eq!(
        T3Api::new(paths)
            .unwrap()
            .ensure_idle_thread("deck-a")
            .unwrap(),
        "idle-a"
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests.last().unwrap().method, "DELETE");
}

#[test]
fn failed_mcp_tool_is_redacted_and_transport_session_is_closed() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    fake_token(&paths);
    let server = MockServer::with_headers(|request| {
        if request.method == "DELETE" {
            return (204, Vec::new(), String::new());
        }
        let result = match request.body["method"].as_str().unwrap() {
            "initialize" => json!({"protocolVersion":"2025-06-18"}),
            "notifications/initialized" => return (202, Vec::new(), String::new()),
            "tools/call" => {
                json!({"isError":true, "content":[{"type":"text", "text":"private fake-test-bearer"}]})
            }
            _ => panic!("unexpected method"),
        };
        (
            200,
            vec![("Mcp-Session-Id".into(), "test-session".into())],
            json!({"jsonrpc":"2.0", "id":request.body["id"], "result":result}).to_string(),
        )
    });
    runtime(&paths, &server.origin);
    let error = T3Api::new(paths)
        .unwrap()
        .ensure_idle_thread("deck-a")
        .unwrap_err();
    assert!(matches!(error, Error::Protocol(_)));
    assert!(!format!("{error:?}").contains("fake-test"));
    assert_eq!(
        server.requests.lock().unwrap().last().unwrap().method,
        "DELETE"
    );
}

#[test]
fn preexisting_generic_bearer_reports_mcp_authentication_failure() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    fake_token(&paths);
    let server = MockServer::new(|_| (401, "private error".into()));
    runtime(&paths, &server.origin);
    assert!(matches!(
        T3Api::new(paths).unwrap().ensure_idle_thread("deck-a"),
        Err(Error::Http { status: 401 })
    ));
}
