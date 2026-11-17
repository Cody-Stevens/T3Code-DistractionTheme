//! Scoped bearer client porting `t3api.T3Api`. No token-bearing type implements Debug/Serialize.

use crate::T3Paths;
use dt_core::{Error, Result};
use reqwest::{blocking::Client, header::HeaderValue, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, time::Duration};

/// Live project from `T3Api.projects`; extra server fields remain available without API churn.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Server project ID.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Workspace root, compared case-insensitively on Windows when finding a project.
    pub workspace_root: String,
    /// None for active projects; projects() filters deleted records.
    #[serde(default)]
    pub deleted_at: Option<String>,
    /// Nonempty when pinned.
    #[serde(default)]
    pub pinned_at: Option<String>,
    /// Current favicon override, if any.
    #[serde(default)]
    pub favicon_path: Option<String>,
    /// Other API fields, without discarding forward-compatible data.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// `T3Api.call` HTTP verb; API does not require arbitrary methods.
#[derive(Clone, Copy, Debug)]
pub enum Method {
    /// Read-only call.
    Get,
    /// Project mutation.
    Post,
}

/// Port `t3api.T3Api`; keep the scoped bearer private and redact all request errors.
/// Do not store/log request Debug output, headers, token contents or response bodies in errors.
pub struct T3Api {
    paths: T3Paths,
    bearer: HeaderValue,
    client: Client,
}

impl T3Api {
    /// Port `T3Api.__init__`: read/trim the supplied owner-only token file; reject empty/missing.
    /// Only orchestration:read/operate are required. Disable redirects/proxies for authenticated
    /// local API traffic and validate the runtime origin before sending the bearer.
    pub fn new(paths: T3Paths) -> Result<Self> {
        let token = std::fs::read_to_string(&paths.token_file)?;
        Self::with_token(paths, &token)
    }

    /// A client using `token` instead of the token file: setup's short-lived MCP session.
    pub fn with_token(paths: T3Paths, token: &str) -> Result<Self> {
        let token = token.trim();
        if token.is_empty() {
            return Err(Error::InvalidInput("empty T3 token"));
        }
        let mut bearer = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| Error::InvalidInput("invalid T3 token"))?;
        bearer.set_sensitive(true);
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| Error::Protocol("could not initialize T3 client"))?;
        Ok(Self {
            paths,
            bearer,
            client,
        })
    }

    /// Port `T3Api.origin`: re-read runtime on every request, returning the current origin.
    pub fn origin(&self) -> Result<String> {
        Ok(crate::read_runtime(&self.paths)?.origin)
    }

    /// Port `T3Api.call`: JSON request/response, null on empty success body, default timeout 10 s.
    /// Path is relative and starts with `/api/`; errors retain HTTP status without secret content.
    /// This blocks on the API worker, never in the audio callback/theme build path.
    pub fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Value> {
        if !path.starts_with("/api/")
            || path.contains("..")
            || !path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_-".contains(&b))
            || timeout.is_zero()
        {
            return Err(Error::InvalidInput("invalid T3 API path or timeout"));
        }
        let url = crate::validate_origin(&self.origin()?)?
            .join(path)
            .map_err(|_| Error::InvalidInput("invalid T3 API path"))?;
        let mut request = self
            .client
            .request(
                match method {
                    Method::Get => reqwest::Method::GET,
                    Method::Post => reqwest::Method::POST,
                },
                url,
            )
            .header(reqwest::header::AUTHORIZATION, self.bearer.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .timeout(timeout);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .map_err(|_| Error::Protocol("T3 request failed"))?;
        if !response.status().is_success() {
            return Err(Error::Http {
                status: response.status().as_u16(),
            });
        }
        let bytes = response
            .bytes()
            .map_err(|_| Error::Protocol("could not read T3 response"))?;
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::Protocol("invalid T3 response JSON"))
    }

    /// Port `T3Api.projects`: GET /api/projects, returning only projects without deletedAt.
    pub fn projects(&self) -> Result<Vec<Project>> {
        let response = self.call(Method::Get, "/api/projects", None, Duration::from_secs(10))?;
        let projects: Vec<Project> = serde_json::from_value(
            response
                .get("projects")
                .cloned()
                .ok_or(Error::Protocol("missing T3 projects"))?,
        )
        .map_err(|_| Error::Protocol("invalid T3 projects"))?;
        Ok(projects
            .into_iter()
            .filter(|p| p.deleted_at.as_deref().is_none_or(str::is_empty))
            .collect())
    }

    /// Port `T3Api.find_project`: prefer matching normalized workspace root, then exact title.
    pub fn find_project(
        &self,
        title: &str,
        workspace_root: Option<&Path>,
    ) -> Result<Option<Project>> {
        let projects = self.projects()?;
        if let Some(root) = workspace_root {
            let root = normalized_root(&root.to_string_lossy());
            if let Some(project) = projects
                .iter()
                .find(|p| normalized_root(&p.workspace_root) == root)
            {
                return Ok(Some(project.clone()));
            }
        }
        Ok(projects.into_iter().find(|p| p.title == title))
    }

    /// Port `T3Api.mutate`: POST /api/projects/mutate with fresh `glowtube:<uuid>` commandId.
    pub fn mutate(&self, mutation: Value) -> Result<Value> {
        let mut mutation = mutation
            .as_object()
            .cloned()
            .ok_or(Error::InvalidInput("project mutation must be an object"))?;
        if !matches!(
            mutation.get("type").and_then(Value::as_str),
            Some("project.create" | "project.update" | "project.delete")
        ) {
            return Err(Error::InvalidInput("invalid project mutation type"));
        }
        mutation.insert(
            "commandId".into(),
            Value::String(format!("glowtube:{}", uuid::Uuid::new_v4())),
        );
        self.call(
            Method::Post,
            "/api/projects/mutate",
            Some(&Value::Object(mutation)),
            Duration::from_secs(10),
        )
    }

    /// Port `T3Api.create_project`: project.create with a fresh UUID, title and workspaceRoot.
    pub fn create_project(&self, title: &str, workspace_root: &Path) -> Result<Project> {
        let response = self.mutate(serde_json::json!({
            "type": "project.create", "projectId": uuid::Uuid::new_v4().to_string(),
            "title": title, "workspaceRoot": workspace_root.to_string_lossy(),
        }))?;
        serde_json::from_value(response).map_err(|_| Error::Protocol("invalid created T3 project"))
    }

    /// Port `T3Api.update_project`: project.update with projectId plus provided server fields.
    /// `faviconPath` is an absolute local path; never publish private files as favicon assets.
    pub fn update_project(
        &self,
        project_id: &str,
        fields: BTreeMap<String, Value>,
    ) -> Result<Value> {
        if fields
            .keys()
            .any(|key| matches!(key.as_str(), "type" | "projectId" | "commandId"))
        {
            return Err(Error::InvalidInput("reserved project update field"));
        }
        let mut mutation: serde_json::Map<String, Value> = fields.into_iter().collect();
        mutation.insert("type".into(), Value::String("project.update".into()));
        mutation.insert("projectId".into(), Value::String(project_id.into()));
        self.mutate(Value::Object(mutation))
    }

    /// Ensure an unsettled, unsnoozed idle sidebar thread through T3's HTTP MCP transport.
    /// Source-verified: t3_thread_launch omits message/attachments, so no turn is dispatched.
    /// Requires a subject=mcp-client session (with only the orchestration scopes). T3 accepts those
    /// only on /mcp, so setup issues a short-lived one for this call (see `setup::setup_decks_with`);
    /// the helper's generic token gets 401 here and keeps working for the projects API.
    pub fn ensure_idle_thread(&self, project_id: &str) -> Result<String> {
        let mut session = McpSession::connect(self)?;
        let listed = session.tool(
            "t3_thread_list",
            serde_json::json!({
                "projectId": project_id, "statuses": ["idle"], "settled": false,
                "snoozed": false, "includeSubagents": false, "limit": 1,
            }),
        )?;
        let threads = listed
            .get("threads")
            .and_then(Value::as_array)
            .ok_or(Error::Protocol("invalid MCP thread list"))?;
        if let Some(thread) = threads.first() {
            if thread.get("projectId").and_then(Value::as_str) != Some(project_id)
                || thread.get("status").and_then(Value::as_str) != Some("idle")
            {
                return Err(Error::Protocol("invalid idle thread identity"));
            }
            return thread
                .get("threadId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or(Error::Protocol("missing idle thread ID"));
        }
        let launched = session.tool(
            "t3_thread_launch",
            serde_json::json!({
                "projectId": project_id, "title": "GlowTube video",
                "workspaceStrategy": { "type": "root" },
                "clientRequestId": format!("glowtube-idle:{}", uuid::Uuid::new_v4()),
            }),
        )?;
        if launched.get("runId") != Some(&Value::Null)
            || launched.get("projectId").and_then(Value::as_str) != Some(project_id)
        {
            return Err(Error::Protocol("MCP did not create an idle thread"));
        }
        launched
            .get("threadId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .ok_or(Error::Protocol("missing created idle thread ID"))
    }
}

// Port the installed CLI's AcpMcpStdioBridge.mcpToolSession HTTP handshake. Keep all credential
// and session state private, close the transport session on every success/error path, and never
// echo MCP error messages (they may contain private request data).
struct McpSession<'a> {
    api: &'a T3Api,
    endpoint: reqwest::Url,
    session_id: Option<HeaderValue>,
    protocol: HeaderValue,
    next_id: u64,
}

impl<'a> McpSession<'a> {
    fn connect(api: &'a T3Api) -> Result<Self> {
        let endpoint = crate::validate_origin(&api.origin()?)?
            .join("/mcp")
            .map_err(|_| Error::Protocol("invalid MCP endpoint"))?;
        let mut session = Self {
            api,
            endpoint,
            session_id: None,
            protocol: HeaderValue::from_static("2025-06-18"),
            next_id: 0,
        };
        let initialized = session.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "glowtube-setup", "version": env!("CARGO_PKG_VERSION")},
            }),
        )?;
        session.protocol = HeaderValue::from_str(
            initialized
                .get("protocolVersion")
                .and_then(Value::as_str)
                .ok_or(Error::Protocol("missing MCP protocol version"))?,
        )
        .map_err(|_| Error::Protocol("invalid MCP protocol version"))?;
        session
            .send(&serde_json::json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))?;
        Ok(session)
    }

    fn headers(&self, method: reqwest::Method) -> reqwest::blocking::RequestBuilder {
        let mut request = self
            .api
            .client
            .request(method, self.endpoint.clone())
            .header(reqwest::header::AUTHORIZATION, self.api.bearer.clone())
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .header("mcp-protocol-version", self.protocol.clone());
        if let Some(id) = &self.session_id {
            request = request.header("mcp-session-id", id.clone());
        }
        request
    }

    fn send(&mut self, message: &Value) -> Result<reqwest::blocking::Response> {
        let response = self
            .headers(reqwest::Method::POST)
            .json(message)
            .send()
            .map_err(|_| Error::Protocol("MCP request failed"))?;
        if !response.status().is_success() {
            return Err(Error::Http {
                status: response.status().as_u16(),
            });
        }
        if let Some(id) = response.headers().get("mcp-session-id") {
            let mut id = id.clone();
            id.set_sensitive(true);
            self.session_id = Some(id);
        }
        Ok(response)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        use std::io::{BufRead, Read};
        self.next_id += 1;
        let id = self.next_id;
        let response = self.send(
            &serde_json::json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}),
        )?;
        let event_stream = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        if !event_stream {
            let mut bytes = Vec::new();
            response.take(1_048_577).read_to_end(&mut bytes)?;
            if bytes.len() > 1_048_576 {
                return Err(Error::Protocol("MCP response exceeds setup limit"));
            }
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| Error::Protocol("invalid MCP JSON"))?;
            return rpc_result(value, id)?.ok_or(Error::Protocol("missing MCP response"));
        }
        let mut reader = std::io::BufReader::new(response);
        let mut data = String::new();
        let mut consumed = 0;
        loop {
            let mut line = String::new();
            let count = reader.by_ref().take(1_048_577).read_line(&mut line)?;
            consumed += count;
            if consumed > 1_048_576 {
                return Err(Error::Protocol("MCP event stream exceeds setup limit"));
            }
            if line.trim().is_empty() && !data.is_empty() {
                let value: Value = serde_json::from_str(&data)
                    .map_err(|_| Error::Protocol("invalid MCP event JSON"))?;
                data.clear();
                if let Some(result) = rpc_result(value, id)? {
                    return Ok(result);
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
            }
            if count == 0 {
                return Err(Error::Protocol("missing MCP event response"));
            }
        }
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        let result = self.request(
            "tools/call",
            serde_json::json!({"name":name, "arguments":arguments}),
        )?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(Error::Protocol("MCP setup tool failed"));
        }
        result
            .get("structuredContent")
            .cloned()
            .ok_or(Error::Protocol("missing MCP setup result"))
    }
}

impl Drop for McpSession<'_> {
    fn drop(&mut self) {
        if self.session_id.is_some() {
            let _ = self.headers(reqwest::Method::DELETE).send();
        }
    }
}

fn rpc_result(envelope: Value, id: u64) -> Result<Option<Value>> {
    if envelope.get("id").and_then(Value::as_u64) != Some(id) {
        return Ok(None);
    }
    if envelope.get("error").is_some() {
        return Err(Error::Protocol("MCP setup RPC failed"));
    }
    envelope
        .get("result")
        .cloned()
        .map(Some)
        .ok_or(Error::Protocol("missing MCP RPC result"))
}

fn normalized_root(root: &str) -> String {
    if cfg!(windows) {
        root.replace('/', "\\").to_lowercase()
    } else {
        root.to_owned()
    }
}
