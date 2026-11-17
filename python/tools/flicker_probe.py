"""Watch a few pixels of the live T3 window very often, to catch one-frame flashes of the theme.

Samples a small patch at the player's centre and one in the chat background every few
milliseconds for a few seconds, and reports frames where the player patch suddenly loses its
picture (near-black and flat), i.e. moments when the theme was not applied.
"""
import sys
import time
from pathlib import Path

import numpy as np
from PIL import ImageGrab

sys.path.insert(0, str(Path(__file__).resolve().parent))
from grab_t3 import find_t3  # noqa: E402

left, top, right, bottom = find_t3()
w, h = right - left, bottom - top
# player centre: middle of the chat column, 40% down the window
px, py = left + int(w * 0.57), top + int(h * 0.42)
samples = []
t_end = time.perf_counter() + float(sys.argv[1] if len(sys.argv) > 1 else 4)
while time.perf_counter() < t_end:
    patch = np.asarray(ImageGrab.grab(bbox=(px - 30, py - 20, px + 30, py + 20), all_screens=True), float)
    samples.append((time.perf_counter(), patch.mean(), patch.std()))
    time.sleep(0.003)
t0 = samples[0][0]
flat = [(round(t - t0, 3), round(m, 1), round(s, 1)) for t, m, s in samples if s < 4]
gaps = np.diff([s[0] for s in samples])
print(f"{len(samples)} samples, every {gaps.mean() * 1000:.1f} ms on average")
print(f"flat (no picture) samples: {len(flat)}", flat[:12])
