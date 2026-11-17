"""Pillow AVIF speed settings at matched size: PSNR and encode time for 8-frame 640x360 clips."""
import io, subprocess, time
from pathlib import Path
import numpy as np
from PIL import Image

SRC = str(Path(__file__).resolve().parents[1] / "cache" / "fDltPLFdkYI.mp4")
W, H, N = 640, 360, 8

def frames(start):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-ss", str(start), "-i", SRC, "-an", "-vf", f"fps=24,scale={W}:{H}",
                          "-frames:v", str(N), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"], capture_output=True, check=True).stdout
    return np.frombuffer(raw, np.uint8).reshape(N, H, W, 3)

def encode(ref, q, speed):
    imgs = [Image.fromarray(f) for f in ref]
    b = io.BytesIO(); t0 = time.perf_counter()
    imgs[0].save(b, "AVIF", save_all=True, append_images=imgs[1:], duration=42, loop=1, quality=q, speed=speed, max_threads=4)
    ms = (time.perf_counter() - t0) * 1000
    im = Image.open(io.BytesIO(b.getvalue())); dec = []
    for i in range(im.n_frames):
        im.seek(i); dec.append(np.asarray(im.convert("RGB")))
    mse = ((np.stack(dec).astype(float) - ref.astype(float)) ** 2).mean()
    return len(b.getvalue()), 10 * np.log10(255 ** 2 / mse), ms

for start in (32, 77, 154):
    ref = frames(start)
    print(f"t={start}s")
    curves = {}
    for speed in (10, 8, 7, 6):
        pts = [encode(ref, q, speed) for q in (20, 30, 40, 50)]
        curves[speed] = pts
        print(f"  speed {speed:2}: " + ", ".join(f"{s // 1000}KB {p:.1f}dB {ms:.0f}ms" for s, p, ms in pts))
    for target in (15000, 20000):
        row = []
        for speed, pts in curves.items():
            xs, ys = zip(*sorted((s, p) for s, p, _ in pts))
            row.append(f"speed {speed}: {np.interp(target, xs, ys):.2f} dB" if xs[0] <= target <= xs[-1] else f"speed {speed}: n/a")
        print(f"  at {target // 1000} KB -> " + ", ".join(row))
