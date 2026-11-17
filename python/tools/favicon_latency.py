"""Time how long the desktop app takes to show a newly published favicon clip.

Pauses nothing; it publishes its own clip (so the helper's next publish will replace it), then samples
the window every ~0.2 s for the magenta outline the theme draws around the clip image."""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.dirname(__file__))
import numpy as np  # noqa: E402
from PIL import ImageGrab  # noqa: E402

import encoder  # noqa: E402
import t3api  # noqa: E402
from grab_t3 import find_t3  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


def visible(box):
    a = np.asarray(ImageGrab.grab(bbox=box, all_screens=True).convert("RGB")).astype(int)
    return ((a[..., 0] > 220) & (a[..., 1] < 40) & (a[..., 2] > 220)).sum() > 1000


def main():
    api = t3api.T3Api()
    pid = api.find_project("GlowTube", ROOT)["id"]
    box = find_t3()
    for seconds, pad in ((30, 4_300_000),):
        path = os.path.join(ROOT, "clips", f"glowtube-clip-latency-{seconds}-{int(time.time())}.svg")
        encoder.encode_clip_svg(os.path.join(ROOT, "cache", "fDltPLFdkYI.mp4"), 60.0, seconds, path, 24, (640, 360),
                                min_bytes=pad)
        size = os.path.getsize(path) / 1e6
        while visible(box):          # wait until the current clip has gone, so we time from a clean start
            time.sleep(0.2)
        hold = float(os.environ.get("HOLD", "0"))
        if hold:
            import urllib.request
            urllib.request.urlopen(f"http://127.0.0.1:47873/holdwrites?v={hold}", timeout=3).read()
            time.sleep(0.3)
        t0 = time.perf_counter()
        api.update_project(pid, faviconPath=path)
        shown = None
        while time.perf_counter() - t0 < 25:
            if visible(box):
                shown = time.perf_counter() - t0
                break
            time.sleep(0.15)
        print(f"{seconds} s clip ({size:.1f} MB): shown after {shown:.2f} s" if shown else f"{seconds} s clip: not shown in 25 s")
        time.sleep(3)


if __name__ == "__main__":
    main()
