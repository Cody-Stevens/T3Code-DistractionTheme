"""Pillow (libaom) vs ffmpeg SVT-AV1 low-delay at matched size: PSNR of 8-frame 640x360 clips."""
import io
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import numpy as np
from PIL import Image

SRC = str(Path(__file__).resolve().parents[1] / "cache" / "fDltPLFdkYI.mp4")
W, H, N = 640, 360, 8


def frames(start):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-ss", str(start), "-i", SRC, "-an", "-vf", f"fps=24,scale={W}:{H}",
                          "-frames:v", str(N), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"], capture_output=True, check=True).stdout
    return np.frombuffer(raw, np.uint8).reshape(N, H, W, 3)


def decode(data):
    im = Image.open(io.BytesIO(data))
    out = []
    for i in range(getattr(im, "n_frames", 1)):
        im.seek(i)
        out.append(np.asarray(im.convert("RGB")))
    return np.stack(out)


def psnr(a, b):
    mse = ((a.astype(float) - b.astype(float)) ** 2).mean()
    return 10 * np.log10(255 ** 2 / mse)


def pillow(ref, q):
    imgs = [Image.fromarray(f) for f in ref]
    b = io.BytesIO()
    t0 = time.perf_counter()
    imgs[0].save(b, "AVIF", save_all=True, append_images=imgs[1:], duration=42, loop=1, quality=q, speed=10, max_threads=4)
    return b.getvalue(), (time.perf_counter() - t0) * 1000


def svt(ref, crf, tmp):
    t0 = time.perf_counter()
    subprocess.run(["ffmpeg", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", "24", "-i", "-",
                    "-pix_fmt", "yuv420p", "-c:v", "libsvtav1", "-preset", "12", "-crf", str(crf),
                    "-svtav1-params", "pred-struct=1", "-f", "avif", "-loop", "1", "-y", tmp],
                   input=ref.tobytes(), capture_output=True, check=True)
    return Path(tmp).read_bytes(), (time.perf_counter() - t0) * 1000


def curve(points, size):
    """PSNR at a given size, interpolated along (size, psnr) points."""
    points = sorted(points)
    xs, ys = [p[0] for p in points], [p[1] for p in points]
    return float(np.interp(size, xs, ys)) if xs[0] <= size <= xs[-1] else None


tmp = str(Path(tempfile.gettempdir()) / "gt_match.avif")
for start in (32, 90, 154):
    ref = frames(start)
    rows = {"pillow": [], "svt": []}
    for q in (15, 25, 35, 45, 55):
        d, ms = pillow(ref, q)
        rows["pillow"].append((len(d), psnr(decode(d), ref), ms))
    for crf in (40, 48, 55, 60, 63):
        d, ms = svt(ref, crf, tmp)
        dec = decode(d)
        rows["svt"].append((len(d), psnr(dec, ref[: len(dec)]), ms, len(dec)))
    print(f"t={start}s")
    for name, pts in rows.items():
        print("  " + name + ": " + ", ".join(f"{p[0] // 1000}KB {p[1]:.1f}dB {p[2]:.0f}ms" + (f" ({p[3]}f)" if len(p) > 3 else "") for p in pts))
    for target in (12000, 18000, 24000):
        a = curve([(p[0], p[1]) for p in rows["pillow"]], target)
        b = curve([(p[0], p[1]) for p in rows["svt"]], target)
        if a and b:
            print(f"  at {target // 1000} KB: pillow {a:.2f} dB, svt {b:.2f} dB, difference {b - a:+.2f} dB")
Path(tmp).unlink(missing_ok=True)
