"""A small client for the running T3 Code server's HTTP API.

Authenticates with a scoped bearer token issued by `t3 auth session issue` (orchestration:read and
orchestration:operate only) and stored in %LOCALAPPDATA%\\GlowTube\\t3-token. The server's address
comes from T3's own runtime file, which changes when T3 restarts.
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
import uuid
from pathlib import Path

T3_HOME = Path(os.environ.get("T3CODE_HOME", Path.home() / ".t3"))
RUNTIME_FILE = T3_HOME / "userdata" / "server-runtime.json"
# Interim location under the security SOP (SEC-3): an owner-only folder outside any working tree.
TOKEN_FILE = Path(os.environ.get("DISTRACTION_T3_TOKEN_FILE",
                                 Path.home() / ".secrets" / "t3code-distraction-theme" / "t3-token"))


class T3Error(RuntimeError):
    pass


class T3Api:
    def __init__(self, token_file: Path = TOKEN_FILE):
        if not token_file.exists():
            raise T3Error(f"no T3 token at {token_file}")
        self.token = token_file.read_text().strip()

    @staticmethod
    def origin() -> str:
        try:
            return json.loads(RUNTIME_FILE.read_text())["origin"]
        except (OSError, ValueError, KeyError) as exc:
            raise T3Error("T3 Code is not running") from exc

    def call(self, method: str, path: str, body: dict | None = None, timeout: float = 10) -> dict:
        req = urllib.request.Request(self.origin() + path, method=method,
                                     data=json.dumps(body).encode() if body is not None else None,
                                     headers={"Authorization": f"Bearer {self.token}",
                                              "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return json.loads(resp.read() or b"null")
        except urllib.error.HTTPError as exc:
            raise T3Error(f"{method} {path}: HTTP {exc.code} {exc.read()[:300]!r}") from exc
        except urllib.error.URLError as exc:
            raise T3Error(f"{method} {path}: {exc.reason}") from exc

    def projects(self) -> list[dict]:
        return [p for p in self.call("GET", "/api/projects")["projects"] if not p.get("deletedAt")]

    def find_project(self, title: str, workspace_root: str | None = None) -> dict | None:
        for p in self.projects():
            if workspace_root and os.path.normcase(p["workspaceRoot"]) == os.path.normcase(workspace_root):
                return p
        return next((p for p in self.projects() if p["title"] == title), None)

    def mutate(self, mutation: dict) -> dict:
        return self.call("POST", "/api/projects/mutate", {"commandId": f"glowtube:{uuid.uuid4()}", **mutation})

    def create_project(self, title: str, workspace_root: str) -> dict:
        return self.mutate({"type": "project.create", "projectId": str(uuid.uuid4()), "title": title,
                            "workspaceRoot": workspace_root})

    def update_project(self, project_id: str, **fields) -> dict:
        return self.mutate({"type": "project.update", "projectId": project_id, **fields})
