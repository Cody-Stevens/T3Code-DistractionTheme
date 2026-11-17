"""Chart how often the favicon video overlay is on screen in the live T3 window.

Samples the window every ~0.4 s for the theme's magenta outline around the clip image ('#' shown,
'.' not shown) and marks each favicon change with the clip's number."""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.dirname(__file__))
import numpy as np  # noqa: E402
from PIL import ImageGrab  # noqa: E402

import t3api  # noqa: E402
from grab_t3 import find_t3  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


def main(seconds: float) -> None:
    l, t, r, b = find_t3()
    api = t3api.T3Api()
    row, t0, last = [], time.perf_counter(), None
    while time.perf_counter() - t0 < seconds:
        a = np.asarray(ImageGrab.grab(bbox=(l, t, r, b), all_screens=True).convert("RGB")).astype(int)
        row.append("#" if ((a[..., 0] > 220) & (a[..., 1] < 40) & (a[..., 2] > 220)).sum() > 1000 else ".")
        if len(row) % 8 == 0:
            fav = api.find_project("GlowTube", ROOT)["faviconPath"].rsplit("-", 1)[-1]
            if fav != last:
                row.append(f"[{fav}]")
                last = fav
        time.sleep(0.3)
    text = "".join(row)
    shown = text.count("#")
    print(text)
    print(f"overlay visible {shown / max(1, shown + text.count('.')):.0%} of samples")


if __name__ == "__main__":
    main(float(sys.argv[1]) if len(sys.argv) > 1 else 90)
