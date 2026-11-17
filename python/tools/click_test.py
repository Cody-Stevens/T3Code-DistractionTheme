"""Click-detection test: every control on a Chromium-rendered card at 100% and 150% scale must be
recognised (centre and off-centre clicks), and random clicks on busy video frames must not be."""
import json
import random
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageGrab

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import card_image as c  # noqa: E402
import glowtube as g  # noqa: E402
import theme_css as t  # noqa: E402

TMP = Path.home() / "AppData/Local/Temp"
items = [c.QueueItem("Polyphia - Playing God (Official Music Video)"), c.QueueItem("Covet - shibuya", "loading"),
         c.QueueItem("CHON - Story")]
card = c.render("Polyphia - POWER IN THE BLOOD (Official Music Video)", "Polyphia  ·  2:40 / 4:01", True, items)
values = {"pr": "0.4", "bs": "0.5", "aa": "#ff8800", **{f"b{i}": f"{(i % 5) / 5:.2f}" for i in range(16)},
          **{f"c{i}": "#ff2bd6" for i in range(4)}}
css = g.STATIC + t.static_values(values) + t.card_height_css(card.height) + t.card_image_css(card)
html = f"""<!doctype html><html><head><style>body{{margin:0;background:#07060b}}
.panel{{position:absolute;left:60px;top:40px;width:{c.WIDTH}px}}
[data-thread-details-card]{{height:90px;background:#1b1530;border-radius:16px}}</style></head>
<body><div class=panel><div data-thread-details-card></div></div>
<script>const s=new CSSStyleSheet(); s.replaceSync({json.dumps(css)}); document.adoptedStyleSheets=[s];</script></body></html>"""
page = TMP / "gt_click_page.html"
page.write_text(html, encoding="utf-8")
edge = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
prof = TMP / "gt-edge-click"
for z in ("1", "1.5"):
    subprocess.run([edge, "--headless=new", "--no-first-run", f"--user-data-dir={prof}", f"--force-device-scale-factor={z}",
                    "--window-size=500,560", f"--screenshot={TMP / f'gt_click_{z}.png'}", "--virtual-time-budget=1000",
                    page.as_uri()], timeout=60, capture_output=True)
shutil.rmtree(prof, ignore_errors=True)
page.unlink()

ox, oy = 60, 40 + 90 + 12
list_top = c.CONTROL_Y + 22 + 18
row_y = lambda i: oy + list_top + i * (c.ROW_H + c.ROW_GAP)  # noqa: E731
targets = {
    "toggle": [(ox + c.PAD + 14, oy + c.CONTROL_Y), (ox + c.PAD + 6, oy + c.CONTROL_Y - 6), (ox + c.PAD + 22, oy + c.CONTROL_Y + 6)],
    "scrub": [(ox + c.BAR_LEFT + 60, oy + c.CONTROL_Y), (ox + c.BAR_LEFT + 200, oy + c.CONTROL_Y + 3)],
    "row0": [(ox + 150, row_y(0) + 12), (ox + 30, row_y(0) + 5), (ox + 240, row_y(0) + 19)],
    "row2": [(ox + 40, row_y(2) + 12), (ox + 200, row_y(2) + 6)],
    "linkplay": [(ox + 60, oy + card.height - c.PAD - 14), (ox + 25, oy + card.height - c.PAD - 22)],
    "linkqueue": [(ox + 230, oy + card.height - c.PAD - 14), (ox + 160, oy + card.height - c.PAD - 7)],
}
ok = total = 0
for z in ("1", "1.5"):
    s = float(z)
    shot = Image.open(TMP / f"gt_click_{z}.png").convert("RGB")

    def grab(bbox=None, all_screens=False, shot=shot):
        canvas = Image.new("RGB", (bbox[2] - bbox[0], bbox[3] - bbox[1]))
        canvas.paste(shot, (-bbox[0], -bbox[1]))
        return canvas

    ImageGrab.grab = grab
    for name, pts in targets.items():
        for x, y in pts:
            hit = g.read_click(round(x * s), round(y * s))
            total += 1
            good = hit is not None and hit[0] == name
            ok += good
            if not good:
                print(f"  MISS scale {z} {name} at ({x},{y}) -> {hit}")
    (TMP / f"gt_click_{z}.png").unlink()
print(f"controls recognised: {ok}/{total}")

# false hits: random clicks on busy video frames shown at the player's 800x450
raw = subprocess.run(["ffmpeg", "-v", "error", "-ss", "70", "-i", str(ROOT / "cache/fDltPLFdkYI.mp4"), "-an", "-vf",
                      "fps=1,scale=800:450", "-frames:v", "40", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
                     capture_output=True, check=True).stdout
frames = np.frombuffer(raw, np.uint8).reshape(-1, 450, 800, 3)
random.seed(1)
false = 0
for i in range(400):
    shot = Image.fromarray(frames[i % len(frames)])

    def grab(bbox=None, all_screens=False, shot=shot):
        canvas = Image.new("RGB", (bbox[2] - bbox[0], bbox[3] - bbox[1]))
        canvas.paste(shot, (-bbox[0], -bbox[1]))
        return canvas

    ImageGrab.grab = grab
    hit = g.read_click(random.randrange(20, 780), random.randrange(20, 430))
    if hit and hit[0] != "scrub":
        false += 1
print(f"false button hits on video: {false}/400")
