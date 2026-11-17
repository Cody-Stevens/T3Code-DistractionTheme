"""Log when T3 is (re)loading each favicon deck, next to the audio time.

Run the helper with GLOWTUBE_SHOW_LOADING=1: the theme then paints a green square (deck a) or a blue
square (deck b) in the window's top-left corner while that deck's favicon has a hidden loader image,
which T3 mounts only while a new favicon URL is loading.

  python tools/load_watch.py <seconds>
"""
import json
import sys
import time
import urllib.request
from pathlib import Path

from PIL import ImageGrab

sys.path.insert(0, str(Path(__file__).resolve().parent))
from grab_t3 import find_t3  # noqa: E402


def main(seconds: float) -> None:
    left, top, _, _ = find_t3()
    state, t0 = {}, time.perf_counter()
    while time.perf_counter() - t0 < seconds:
        img = ImageGrab.grab(bbox=(left, top, left + 48, top + 24), all_screens=True).convert("RGB")
        a, b = img.getpixel((12, 12)), img.getpixel((36, 12))
        now = {"a": a[1] > 200 and a[0] < 60 and a[2] < 60, "b": b[2] > 200 and b[0] < 60 and b[1] < 60}
        for deck, on in now.items():
            if state.get(deck) != on:
                st = json.loads(urllib.request.urlopen("http://127.0.0.1:47873/status", timeout=2).read())
                print(f"{time.strftime('%H:%M:%S')}.{int(time.time() * 1000) % 1000:03d} audio {st['time']:6.1f} "
                      f"deck {deck} {'LOADING' if on else 'shown'}", flush=True)
                state[deck] = on
        time.sleep(0.05)


if __name__ == "__main__":
    main(float(sys.argv[1]) if len(sys.argv) > 1 else 60)
