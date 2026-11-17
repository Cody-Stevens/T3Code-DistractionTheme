"""Chart how often the favicon video covers the player in the live T3 window.

Run the helper with GLOWTUBE_MEASURE=1: the theme then paints the player magenta instead of its own
video, so any magenta on screen means no favicon clip is covering it. Samples every ~0.3 s
('#' covered, '.' gap, 'h' music held for the first clip, not counted) and marks each favicon change as [deck:clip number]."""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.dirname(__file__))
import numpy as np  # noqa: E402
from PIL import ImageGrab  # noqa: E402

import json  # noqa: E402
import urllib.request  # noqa: E402

import t3api  # noqa: E402
from grab_t3 import find_t3  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
DECKS = {"a": ("GlowTube", ROOT), "b": ("GlowTube B", os.path.join(ROOT, "deck-b"))}


def main(seconds: float) -> None:
    l, t, r, b = find_t3()
    api = t3api.T3Api()
    row, t0, last, worst = [], time.perf_counter(), {}, 0
    while time.perf_counter() - t0 < seconds:
        a = np.asarray(ImageGrab.grab(bbox=(l, t, r, b), all_screens=True).convert("RGB")).astype(int)
        magenta = int(((a[..., 0] > 200) & (a[..., 1] < 60) & (a[..., 2] > 200)).sum())
        try:
            held = json.loads(urllib.request.urlopen("http://127.0.0.1:47873/status", timeout=1).read()).get("held")
        except OSError:
            held = False
        row.append("h" if held else "." if magenta > 3000 else "#")
        if magenta <= 3000:
            worst = max(worst, magenta)
        if len(row) % 4 == 0:
            for deck, (title, root) in DECKS.items():
                p = api.find_project(title, root)
                fav = (p or {}).get("faviconPath") or ""
                tag = "blank" if "blank" in fav else fav.rsplit("-", 1)[-1].removesuffix(".svg")
                if last.get(deck) != tag:
                    row.append(f"[{deck}:{tag}]")
                    last[deck] = tag
        time.sleep(0.3)
    text = "".join(row)
    shown, gaps = text.count("#"), text.count(".")
    print(text)
    print(f"covered {shown / max(1, shown + gaps):.1%} of {shown + gaps} samples (largest magenta count while covered: {worst})")


if __name__ == "__main__":
    main(float(sys.argv[1]) if len(sys.argv) > 1 else 90)
