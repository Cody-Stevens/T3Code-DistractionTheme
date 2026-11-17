"""Measure audio/video sync on the live T3 window.

Grabs the player area several times, reads the helper's audio position at the same instant, and
finds which frame of the source video is on screen. Prints video time minus audio time (positive:
video ahead) and the matching error (small means a confident match; the theme's own lower-quality
video and the favicon clip both match the source)."""
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


def status():
    return json.loads(urllib.request.urlopen("http://127.0.0.1:47873/status", timeout=3).read())


def main(samples=6):
    left, top, right, bottom = find_t3()
    w, h = right - left, bottom - top
    # the player: 800x450 at 50% across and 40% down the chat area; take its middle 60% (480x270),
    # which is the middle 60% of the 640x360 source (384x216 at 128,72)
    cx, cy = left + int(w * 0.57), top + int(h * 0.42)
    box = (cx - 240, cy - 135, cx + 240, cy + 135)
    shots = []
    for _ in range(samples):
        t_audio = status()["time"]
        img = ImageGrab.grab(bbox=box, all_screens=True).convert("RGB").resize((96, 54))
        shots.append((t_audio, np.asarray(img, float)))
        time.sleep(0.7)
    video = str(ROOT / "cache" / "fDltPLFdkYI.mp4")
    for t_audio, shot in shots:
        start = max(0.0, t_audio - 2.0)
        raw = subprocess.run(["ffmpeg", "-v", "error", "-ss", f"{start:.2f}", "-i", video, "-an", "-t", "4", "-vf",
                              "fps=24,scale=640:360,crop=384:216:128:72,scale=96:54", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
                             capture_output=True, check=True).stdout
        frames = np.frombuffer(raw, np.uint8).reshape(-1, 54, 96, 3).astype(float)
        errs = [np.abs(f - shot).mean() for f in frames]
        i = int(np.argmin(errs))
        print(f"audio {t_audio:7.2f}s  video {start + i / 24:7.2f}s  offset {start + i / 24 - t_audio:+.2f}s  error {errs[i]:.1f}")


if __name__ == "__main__":
    main()
