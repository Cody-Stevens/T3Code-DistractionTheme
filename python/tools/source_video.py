r"""Find the test video (fDltPLFdkYI) for the sync tools: the Python helper's cache first, then the
Rust helper's (%LOCALAPPDATA%\GlowTube\cache)."""
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def path(video_id: str = "fDltPLFdkYI") -> str:
    candidates = [ROOT / "cache" / f"{video_id}.mp4",
                  Path(os.environ.get("LOCALAPPDATA", Path.home())) / "GlowTube" / "cache" / f"{video_id}.mp4"]
    for candidate in candidates:
        if candidate.exists():
            return str(candidate)
    raise SystemExit(f"{video_id}.mp4 not found; play it once first. Looked in: "
                     + ", ".join(str(c.parent) for c in candidates))
