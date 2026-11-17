"""Measure audio/video sync on the live T3 window against the whole source video.

  python tools/sync_full.py <seconds> [interval]

sync_check.py only searches 2-8 s around the audio position. A favicon clip that loaded late (or a
stale clip left on screen) can be much further off, which looks like a bad match there. This tool
matches each grab of the player against every frame of the source (at 8 fps) and prints what part
of the video is on screen, the offset from the audio, the match error (under ~15 is a confident
match) and which favicon deck is on top. Needs the helper's control port and cache/<id>.mp4."""
import json
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

import numpy as np
from PIL import ImageGrab

sys.path.insert(0, str(Path(__file__).resolve().parent))
from grab_t3 import find_t3  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
FPS = 8


def status() -> dict:
    return json.loads(urllib.request.urlopen("http://127.0.0.1:47873/status", timeout=3).read())


def main(seconds: float, interval: float) -> None:
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", str(ROOT / "cache" / "fDltPLFdkYI.mp4"), "-an", "-vf",
                          f"fps={FPS},scale=640:360,crop=384:216:128:72,scale=48:27", "-f", "rawvideo",
                          "-pix_fmt", "rgb24", "-"], capture_output=True, check=True).stdout
    src = np.frombuffer(raw, np.uint8).reshape(-1, 27, 48, 3).astype(float)
    left, top, right, bottom = find_t3()
    cx, cy = left + int((right - left) * 0.57), top + int((bottom - top) * 0.42)
    box = (cx - 240, cy - 135, cx + 240, cy + 135)
    offsets, t0 = [], time.perf_counter()
    while time.perf_counter() - t0 < seconds:
        st = status()
        shot = np.asarray(ImageGrab.grab(bbox=box, all_screens=True).convert("RGB").resize((48, 27)), float)
        errs = np.abs(src - shot).mean(axis=(1, 2, 3))
        i = int(np.argmin(errs))
        off = i / FPS - st["time"]
        if errs[i] < 15 and not st.get("held"):
            offsets.append(off)
        print(f"{time.strftime('%H:%M:%S')} audio {st['time']:6.1f}  on screen {i / FPS:6.1f}  offset {off:+6.2f}  "
              f"err {errs[i]:5.1f}  front {st.get('front')}", flush=True)
        time.sleep(interval)
    if offsets:
        a = np.abs(offsets)
        print(f"confident samples {len(a)}: |offset| median {np.median(a):.2f}s, within 0.25 s {np.mean(a <= 0.25):.0%}, "
              f"worst {a.max():.2f}s")


if __name__ == "__main__":
    main(float(sys.argv[1]) if len(sys.argv) > 1 else 60, float(sys.argv[2]) if len(sys.argv) > 2 else 1.0)
