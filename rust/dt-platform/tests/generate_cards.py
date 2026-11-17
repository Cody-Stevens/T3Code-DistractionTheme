"""Regenerate synthetic card fixtures; no browser, live window, or downloaded media.

Requires Pillow and numpy, as does the Python reference. Cargo tests use the checked-in RLE
pixels and do not require Python. Store only each target's 1200x17 strip to keep fixtures small.
"""
import base64
import io
from itertools import groupby
from pathlib import Path
import sys

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "python"))
import card_image as c

items = [c.QueueItem("Polyphia - Playing God (Official Music Video)"),
         c.QueueItem("Covet - shibuya", "loading"), c.QueueItem("CHON - Story")]
card = c.render("Polyphia - POWER IN THE BLOOD (Official Music Video)",
                "Polyphia  ·  2:40 / 4:01", True, items)
image = Image.open(io.BytesIO(base64.b64decode(card.image))).convert("RGB")
# The CSS live layer paints a 214x6 progress bar above the card at 40% progress.
d = ImageDraw.Draw(image)
left, top = c.BAR_LEFT * 2, (c.CONTROL_Y - c.BAR_HEIGHT // 2) * 2
right = (c.WIDTH - c.PAD) * 2
d.rectangle((left, top, right - 1, top + c.BAR_HEIGHT * 2 - 1), fill=c.MARK_TRACK)
d.rectangle((left, top, left + round((right-left) * .4) - 1, top+c.BAR_HEIGHT*2-1), fill=c.MARK_FILL)
list_top = c.CONTROL_Y + 22 + 18
row_y = lambda i: list_top + i * (c.ROW_H + c.ROW_GAP)
targets = {
    "toggle": [(c.PAD+14,c.CONTROL_Y),(c.PAD+6,c.CONTROL_Y-6),(c.PAD+22,c.CONTROL_Y+6)],
    "scrub": [(c.BAR_LEFT+60,c.CONTROL_Y),(c.BAR_LEFT+200,c.CONTROL_Y+3)],
    "row0": [(150,row_y(0)+12),(30,row_y(0)+5),(240,row_y(0)+19)],
    "row2": [(40,row_y(2)+12),(200,row_y(2)+6)],
    "linkplay": [(60,card.height-c.PAD-14),(25,card.height-c.PAD-22)],
    "linkqueue": [(230,card.height-c.PAD-14),(160,card.height-c.PAD-7)],
}
out = Path(__file__).parent / "fixtures"
out.mkdir(exist_ok=True)
for scale in (1, 1.5):
    resized = image.resize((round(c.WIDTH*scale),round(card.height*scale)), Image.Resampling.LANCZOS)
    lines = ["# Synthetic Python card strips, RGB RLE: target then count:rrggbb runs (1200x17)."]
    for name, points in targets.items():
        for x,y in points:
            x,y = round(x*scale),round(y*scale)
            strip = Image.new("RGB",(1200,17))
            strip.paste(resized,(600-x,8-y))
            runs = [f"{sum(1 for _ in values)}:{r:02x}{g:02x}{b:02x}"
                    for (r,g,b),values in groupby(strip.getdata())]
            lines.append(name + " " + " ".join(runs))
    (out / f"cards-{int(scale*100)}.rle").write_text("\n".join(lines)+"\n",encoding="ascii")
