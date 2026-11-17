"""Draws the Now Playing card as an image.

A theme cannot lay out a list (CSS strings cannot hold line breaks without a backslash, which T3
strips), so the helper draws the card's static parts: title, time, play/pause, the Up Next list
and the two link buttons. CSS stacks the live spectrum and progress bar on top.

Every clickable part is filled with its own exact marker color. When a left click lands inside
T3, the helper reads the pixels under the cursor and matches them against `MARKS`.
"""

from __future__ import annotations

import base64
import io
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

WIDTH = 276            # CSS pixels: the width of T3's thread details card
SCALE = 2              # drawn at 2x so text stays sharp on scaled displays
PAD = 12
BG = (17, 10, 29)

# Layout, in CSS pixels. CSS draws the spectrum and progress bar into the gaps left here.
SPECTRUM_TOP, SPECTRUM_BOTTOM = 50, 94
CONTROL_Y = 118                         # centre line of the play button and progress bar
BAR_LEFT, BAR_HEIGHT = 50, 6
QUEUE_ROWS = 4
ROW_H, ROW_GAP = 24, 3

# Marker colors: every pair differs by at least 32 in some channel, twice the match tolerance.
MARKS = {
    "toggle": (0x58, 0x18, 0x38),
    "linkplay": (0x18, 0x58, 0x38),
    "linkqueue": (0x38, 0x18, 0x58),
    "row0": (0x58, 0x38, 0x18),
    "row1": (0x18, 0x38, 0x58),
    "row2": (0x38, 0x58, 0x18),
    "row3": (0x58, 0x58, 0x18),
}
MARK_TRACK = (0x4a, 0x4a, 0x66)   # progress bar, not yet played (drawn by CSS)
MARK_FILL = (0xe6, 0xe6, 0xff)    # progress bar, played (drawn by CSS)

_FONTS = Path("C:/Windows/Fonts")


def hexcolor(rgb) -> str:
    return "#%02x%02x%02x" % tuple(rgb)


def _font(name: str, size: float) -> ImageFont.FreeTypeFont:
    try:
        return ImageFont.truetype(str(_FONTS / name), round(size * SCALE))
    except OSError:
        return ImageFont.load_default(round(size * SCALE))


TITLE = _font("seguisb.ttf", 13)
BODY = _font("segoeui.ttf", 11.5)
SMALL = _font("seguisb.ttf", 10.5)


@dataclass
class QueueItem:
    title: str
    state: str = ""        # "", "loading" or "failed"


@dataclass
class Card:
    image: str             # base64 WebP
    height: int            # CSS pixels


def _fit(draw: ImageDraw.ImageDraw, text: str, font, width: float) -> str:
    if draw.textlength(text, font=font) <= width:
        return text
    while text and draw.textlength(text + "…", font=font) > width:
        text = text[:-1]
    return text.rstrip() + "…"


def render(title: str, subtitle: str, playing: bool, upcoming: list[QueueItem]) -> Card:
    rows = upcoming[:QUEUE_ROWS]
    queue_top = CONTROL_Y + 22
    list_top = queue_top + 18
    list_h = max(1, len(rows)) * (ROW_H + ROW_GAP)
    buttons_top = list_top + list_h + 6
    height = buttons_top + 28 + PAD

    s = SCALE
    img = Image.new("RGB", (WIDTH * s, height * s), BG)
    d = ImageDraw.Draw(img)
    inner = WIDTH - 2 * PAD

    d.text((PAD * s, 9 * s), _fit(d, title, TITLE, inner * s), font=TITLE, fill="white")
    d.text((PAD * s, 29 * s), _fit(d, subtitle, BODY, inner * s), font=BODY, fill=(201, 195, 221))

    # play/pause disc
    cx, cy, r = PAD + 14, CONTROL_Y, 14
    d.ellipse(((cx - r) * s, (cy - r) * s, (cx + r) * s, (cy + r) * s), fill=MARKS["toggle"])
    if playing:
        for x in (cx - 5, cx + 1.5):
            d.rounded_rectangle((x * s, (cy - 6) * s, (x + 3.5) * s, (cy + 6) * s), radius=s, fill="white")
    else:
        d.polygon([((cx - 4) * s, (cy - 7) * s), ((cx - 4) * s, (cy + 7) * s), ((cx + 7) * s, cy * s)], fill="white")

    d.text((PAD * s, queue_top * s), "UP NEXT", font=SMALL, fill=(154, 147, 184))
    if not rows:
        d.text((PAD * s, (list_top + 4) * s), "Nothing queued", font=BODY, fill=(138, 132, 166))
    for i, item in enumerate(rows):
        top = list_top + i * (ROW_H + ROW_GAP)
        d.rounded_rectangle((PAD * s, top * s, (WIDTH - PAD) * s, (top + ROW_H) * s), radius=6 * s,
                            fill=MARKS[f"row{i}"])
        suffix = {"loading": "  · loading", "failed": "  · failed"}.get(item.state, "")
        label = _fit(d, f"{i + 1}.  {item.title}", BODY, (inner - 16) * s - d.textlength(suffix, font=BODY))
        d.text(((PAD + 8) * s, (top + 4) * s), label + suffix, font=BODY, fill="white")

    half = (inner - 8) / 2
    # Segoe UI has no play glyph, so the icons are drawn as shapes
    for left, mark, label in ((PAD, "linkplay", "Play copied link"), (PAD + half + 8, "linkqueue", "Queue copied link")):
        d.rounded_rectangle((left * s, buttons_top * s, (left + half) * s, (buttons_top + 28) * s), radius=8 * s,
                            fill=MARKS[mark])
        w = d.textlength(label, font=SMALL) + 13 * s
        x0, cy = (left + half / 2) * s - w / 2, (buttons_top + 14) * s
        if mark == "linkplay":
            d.polygon([(x0, cy - 4.5 * s), (x0, cy + 4.5 * s), (x0 + 8 * s, cy)], fill="white")
        else:
            d.rectangle((x0, cy - 1 * s, x0 + 8 * s, cy + 1 * s), fill="white")
            d.rectangle((x0 + 3 * s, cy - 4 * s, x0 + 5 * s, cy + 4 * s), fill="white")
        d.text((x0 + 13 * s, (buttons_top + 7) * s), label, font=SMALL, fill="white")

    return Card(_encode(img), height)


def _encode(img: Image.Image) -> str:
    """Lossless WebP over a small palette: about a third of the size of full color. The marker
    colors cover large flat areas, so they become palette entries unchanged; check anyway."""
    import numpy as np
    for colors in (16, 32, 64):
        small = img.quantize(colors=colors, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE).convert("RGB")
        px = np.asarray(small).reshape(-1, 3)
        if all((px == mark).all(axis=1).any() for mark in MARKS.values() if (np.asarray(img).reshape(-1, 3) == mark).all(axis=1).any()):
            break
    buf = io.BytesIO()
    small.save(buf, "WEBP", lossless=True, method=4)
    return base64.b64encode(buf.getvalue()).decode()
