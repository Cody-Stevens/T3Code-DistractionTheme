//! Scoped bearer client porting `t3api.T3Api`. No token-bearing type implements Debug/Serialize.

use crate::T3Paths;
use dt_core::Result;
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
    _private: (),
}

impl T3Api {
    /// Port `T3Api.__init__`: read/trim the supplied owner-only token file; reject empty/missing.
    /// Only orchestration:read/operate are required. Disable redirects/proxies for authenticated
    /// local API traffic and validate the runtime origin before sending the bearer.
    pub fn new(_paths: T3Paths) -> Result<Self> {
        todo!("read scoped token into private state")
    }

    /// Port `T3Api.origin`: re-read runtime on every request, returning the current origin.
    pub fn origin(&self) -> Result<String> {
        todo!("resolve live API origin")
    }

    /// Port `T3Api.call`: JSON request/response, null on empty success body, default timeout 10 s.
    /// Path is relative and starts with `/api/`; errors retain HTTP status without secret content.
    /// This blocks on the API worker, never in the audio callback/theme build path.
    pub fn call(
        &self,
        _method: Method,
        _path: &str,
        _body: Option<&Value>,
        _timeout: Duration,
    ) -> Result<Value> {
        todo!("authenticated JSON request")
    }

    /// Port `T3Api.projects`: GET /api/projects, returning only projects without deletedAt.
    pub fn projects(&self) -> Result<Vec<Project>> {
        todo!("list active projects")
    }

    /// Port `T3Api.find_project`: prefer matching normalized workspace root, then exact title.
    pub fn find_project(
        &self,
        _title: &str,
        _workspace_root: Option<&Path>,
    ) -> Result<Option<Project>> {
        todo!("find project")
    }

    /// Port `T3Api.mutate`: POST /api/projects/mutate with fresh `glowtube:<uuid>` commandId.
    pub fn mutate(&self, _mutation: Value) -> Result<Value> {
        todo!("send project mutation")
    }

    /// Port `T3Api.create_project`: project.create with a fresh UUID, title and workspaceRoot.
    pub fn create_project(&self, _title: &str, _workspace_root: &Path) -> Result<Project> {
        todo!("create project")
    }

    /// Port `T3Api.update_project`: project.update with projectId plus provided server fields.
    /// `faviconPath` is an absolute local path; never publish private files as favicon assets.
    pub fn update_project(
        &self,
        _project_id: &str,
        _fields: BTreeMap<String, Value>,
    ) -> Result<Value> {
        todo!("update project")
    }
}
