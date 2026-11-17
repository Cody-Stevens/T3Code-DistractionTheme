"""GlowTube service: keeps the player running whenever T3 Code is running.

  python service.py            run the service (one instance per user; meant to start at login)
  python service.py setup      do the one-time setup now and exit

Every few seconds it checks whether T3 Code's server is alive (T3's own runtime file names its
process). When T3 starts, it runs setup and launches the player (`glowtube.py play`); when T3 exits,
it stops the player; if the player crashes while T3 is up, it restarts it, backing off on repeated
crashes. Setup makes everything a fresh install needs, skipping what already exists:

  1. a T3 access token for the player, scoped to orchestration:read and orchestration:operate,
     stored in ~\\.secrets\\t3code-distraction-theme\\t3-token, readable only by this user;
  2. the "GlowTube" project, rooted at this folder and pinned, whose favicon carries the video;
  3. the GlowTube theme file, selected as T3's theme.

Logs go to %LOCALAPPDATA%\\GlowTube\\service.log and player.log.
"""

from __future__ import annotations

import ctypes
import json
import os
import subprocess
import sys
import time
import urllib.request
from ctypes import wintypes
from pathlib import Path

import t3api

HERE = Path(__file__).resolve().parent
STATE = Path(os.environ.get("LOCALAPPDATA", Path.home())) / "GlowTube"
LOG = STATE / "service.log"
PLAYER_LOG = STATE / "player.log"
T3_CLI = t3api.T3_HOME / "bin" / ("t3.cmd" if os.name == "nt" else "t3")
SETTINGS = t3api.T3_HOME / "userdata" / "settings.json"
CTL = "http://127.0.0.1:47873"
POLL = 3.0

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.OpenProcess.restype = wintypes.HANDLE


def log(msg: str) -> None:
    STATE.mkdir(parents=True, exist_ok=True)
    line = f"{time.strftime('%Y-%m-%d %H:%M:%S')} {msg}"
    print(line, flush=True)
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(line + "\n")


def pid_alive(pid: int) -> bool:
    handle = kernel32.OpenProcess(0x1000, False, pid)        # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        return False
    try:
        code = wintypes.DWORD()
        return bool(kernel32.GetExitCodeProcess(handle, ctypes.byref(code))) and code.value == 259  # STILL_ACTIVE
    finally:
        kernel32.CloseHandle(handle)


def t3_running() -> bool:
    try:
        runtime = json.loads(t3api.RUNTIME_FILE.read_text())
    except (OSError, ValueError):
        return False
    return pid_alive(int(runtime.get("pid", 0)))


# ---------------------------------------------------------------- setup


def ensure_token() -> None:
    if t3api.TOKEN_FILE.exists() and t3api.TOKEN_FILE.read_text().strip():
        return
    t3api.TOKEN_FILE.parent.mkdir(parents=True, exist_ok=True)
    out = subprocess.run([str(T3_CLI), "auth", "session", "issue", "--scope", "orchestration:read",
                          "--scope", "orchestration:operate", "--ttl", "90d", "--label", "GlowTube helper", "--json"],
                         capture_output=True, text=True, timeout=60)
    token = json.loads(out.stdout).get("token") if out.returncode == 0 else None
    if not token:
        raise RuntimeError(f"could not issue a T3 token: {out.stderr.strip()[:300]}")
    t3api.TOKEN_FILE.write_text(token)
    user = os.environ.get("USERNAME", "")
    if user:   # owner-only, as the security SOP requires for interim plaintext secrets
        subprocess.run(["icacls", str(t3api.TOKEN_FILE.parent), "/inheritance:r", "/grant:r", f"{user}:(OI)(CI)F"],
                       capture_output=True)
        subprocess.run(["icacls", str(t3api.TOKEN_FILE), "/inheritance:r", "/grant:r", f"{user}:F"], capture_output=True)
    log("issued the T3 token")


def ensure_project(api: t3api.T3Api) -> dict:
    project = api.find_project("GlowTube", str(HERE))
    if project is None:
        project = api.create_project("GlowTube", str(HERE))
        log(f"created the GlowTube project {project['id']}")
    if not project.get("pinnedAt"):
        api.update_project(project["id"], pinned=True)
    return project


def ensure_theme() -> None:
    subprocess.run([sys.executable, str(HERE / "glowtube.py"), "idle"], check=True, timeout=120, cwd=HERE)
    try:
        settings = json.loads(SETTINGS.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        log("could not read T3's settings; select the GlowTube theme in T3's appearance settings")
        return
    if settings.get("defaultTheme") == "glowtube":
        return
    stamp = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())
    settings.update(defaultTheme="glowtube", defaultThemeSetAt=stamp, defaultAppearance="dark",
                    defaultAppearanceSetAt=stamp)
    tmp = SETTINGS.with_name(SETTINGS.name + ".glowtube-tmp")
    tmp.write_text(json.dumps(settings, indent=2), encoding="utf-8")
    os.replace(tmp, SETTINGS)                 # T3 watches this file and applies the change
    log("selected the GlowTube theme")


def setup() -> None:
    ensure_token()
    api = t3api.T3Api()
    ensure_project(api)
    ensure_theme()


# ---------------------------------------------------------------- player process


def start_player() -> subprocess.Popen:
    STATE.mkdir(parents=True, exist_ok=True)
    out = open(PLAYER_LOG, "a", encoding="utf-8")
    flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    return subprocess.Popen([sys.executable, "-u", str(HERE / "glowtube.py"), "play"], cwd=HERE, stdout=out,
                            stderr=subprocess.STDOUT, creationflags=flags)


def stop_player(proc: subprocess.Popen) -> None:
    try:
        urllib.request.urlopen(f"{CTL}/quit", timeout=3).read()
        proc.wait(timeout=10)
    except Exception:
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True)


def single_instance() -> bool:
    kernel32.CreateMutexW(None, False, "Local\\GlowTubeService")
    return ctypes.get_last_error() != 183     # ERROR_ALREADY_EXISTS


def serve() -> None:
    if not single_instance():
        print("GlowTube service is already running", flush=True)
        return
    log("service started")
    player: subprocess.Popen | None = None
    crashes, next_start = 0, 0.0
    while True:
        running = t3_running()
        if running and player is None and time.monotonic() >= next_start:
            try:
                setup()
                player = start_player()
                log(f"player started (pid {player.pid})")
            except Exception as exc:
                log(f"setup failed: {exc}")
                next_start = time.monotonic() + 30
        elif player is not None and player.poll() is not None:
            code, player = player.returncode, None
            if running:
                crashes += 1
                delay = min(60, 2 ** crashes)
                next_start = time.monotonic() + delay
                log(f"player exited with code {code}; restarting in {delay}s")
            else:
                log("player stopped")
        elif player is not None and not running:
            log("T3 closed; stopping the player")
            stop_player(player)
            player, crashes = None, 0
        elif player is not None and crashes and time.monotonic() > next_start + 300:
            crashes = 0                      # five calm minutes reset the back-off
        time.sleep(POLL)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "setup":
        setup()
    else:
        serve()
