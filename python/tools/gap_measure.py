"""Measure the empty space between chat text and the player in T3 screenshots.

  python tools/gap_measure.py <player-top> <player-bottom> <png> [...]

Player edges are in screenshot pixels (read them off one screenshot; they do not move while the
window keeps its size). Scans the text column (the player's width) for the nearest row above the
player and below it that holds text-bright pixels (all channels > 150, so the orange glow and
video light do not count) and prints both gaps.
"""
import sys

from PIL import Image


def bright(px, y, x0, x1):
    # Text is near-grey and lighter than the dark chat (the multiply layer dims it to ~60-200);
    # the glow is saturated orange, so a grey test separates them.
    def text(c):
        return min(c) > 50 and max(c) - min(c) < 45
    return sum(1 for x in range(x0, x1, 2) if text(px[x, y][:3])) >= 3


def main():
    top, bot = int(sys.argv[1]), int(sys.argv[2])
    for path in sys.argv[3:]:
        im = Image.open(path).convert("RGB")
        px = im.load()
        x0, x1 = int(im.width * 0.38), int(im.width * 0.76)    # chat column under the player
        above = next((top - y for y in range(top - 3, 50, -1) if bright(px, y, x0, x1)), None)
        below = next((y - bot for y in range(bot + 3, im.height - 200) if bright(px, y, x0, x1)), None)
        print(f"{path}: gap above {above} px, gap below {below} px")


if __name__ == "__main__":
    main()
