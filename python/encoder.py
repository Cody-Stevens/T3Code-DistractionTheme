"""Video clip encoding for worker processes.

Kept apart from glowtube.py so worker processes start without loading audio, Windows input or the
theme code. libavif at speed 6 gives about 2 dB more than speed 10 at the same size, but takes
~200-400 ms for an 8-frame 640x360 clip, longer than the 250 ms between writes; clips are
independent, so several processes encode them ahead of time in parallel.
"""

from __future__ import annotations

import base64
import io
import math

from PIL import Image


def warm_up() -> None:
    """Load the AVIF encoder in a fresh worker process, so the first real clip doesn't pay for it."""
    Image.new("RGB", (16, 16)).save(io.BytesIO(), format="AVIF", speed=10)

WIDTHS = (640, 512, 384)


def encode_clip_svg(video: str, start: float, seconds: float, out_path: str, fps: int, size: tuple[int, int],
                    quality: int = 60, speed: int = 8, threads: int = 6, min_bytes: int = 4_300_000) -> tuple[str, int]:
    """Decode `seconds` of `video` from `start`, encode it as an animated AVIF and wrap it in an SVG.

    T3 shows a project's favicon with a real <img>. SVG favicons over its 32,768-character cache
    limit are loaded from the file itself instead of a downscaled thumbnail, so this file carries
    full-quality video without passing through the theme's 50,000-character stylesheet."""
    import subprocess

    w, h = size
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-ss", f"{start:.3f}", "-i", video, "-an", "-t", f"{seconds:.3f}", "-vf",
         f"fps={fps},scale={w}:{h}:force_original_aspect_ratio=increase:flags=area,crop={w}:{h}",
         "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        capture_output=True, check=True).stdout
    count = len(raw) // (w * h * 3)
    if count == 0:
        raise ValueError("no frames decoded")
    frames = [Image.frombytes("RGB", size, raw[i * w * h * 3:(i + 1) * w * h * 3]) for i in range(count)]
    # Whole-millisecond frame times that add up exactly to the frame rate (41, 42, 42 ms for 24 fps),
    # so a long clip stays in step with the audio instead of drifting by the rounding.
    durations = [round((i + 1) * 1000 / fps) - round(i * 1000 / fps) for i in range(count)]
    buf = io.BytesIO()
    frames[0].save(buf, "AVIF", save_all=count > 1, append_images=frames[1:], duration=durations, loop=1,
                   quality=quality, speed=speed, max_threads=threads)
    data = base64.b64encode(buf.getvalue()).decode()
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">'
           f'<image href="data:image/avif;base64,{data}" width="{w}" height="{h}"/></svg>')
    # T3's icon loader stops at once on anything over 4 MiB instead of downloading it in full first
    # (only to find it too big to cache), so pad small clips past that.
    if len(svg) < min_bytes:
        svg = svg[:-6] + "<!--" + "." * (min_bytes - len(svg)) + "--></svg>"
    with open(out_path, "w", encoding="ascii") as f:
        f.write(svg)
    return out_path, count


def _encode(frames: list[Image.Image], quality: int, speed: int, frame_ms: int) -> bytes:
    buf = io.BytesIO()
    frames[0].save(buf, "AVIF", save_all=len(frames) > 1, append_images=frames[1:], duration=frame_ms, loop=1,
                   quality=quality, speed=speed, max_threads=2)
    return buf.getvalue()


def _b64_len(n: int) -> int:
    return 4 * math.ceil(n / 3)


def encode_to_fit(raw: bytes, size: tuple[int, int], count: int, room: int, frame_ms: int) -> tuple[str, int, int] | None:
    """The best speed-6 animated AVIF of these frames whose base64 fits in `room` characters.

    Two fast speed-10 encodes measure how much this clip costs at each quality (size grows about
    exponentially with quality), which picks the quality for one speed-6 encode aimed at 85% of the
    room; one safer retry if it still comes out too big. Returns (base64, width, quality) or None."""
    w, h = size
    full = [Image.frombytes("RGB", size, raw[i * w * h * 3:(i + 1) * w * h * 3]) for i in range(count)]
    target = 0.85 * room
    for width in WIDTHS:
        frames = full if width == w else [f.resize((width, round(width * 9 / 16)), Image.BILINEAR) for f in full]
        lo_q, hi_q = 20, 45
        lo, hi = _b64_len(len(_encode(frames, lo_q, 10, frame_ms))), _b64_len(len(_encode(frames, hi_q, 10, frame_ms)))
        if lo > room and width != WIDTHS[-1]:
            continue                      # too complex for this width even at low quality
        slope = (math.log(hi) - math.log(lo)) / (hi_q - lo_q) or 0.03
        quality = int(max(10, min(70, lo_q + (math.log(target) - math.log(lo)) / slope)))
        for attempt in (quality, quality - 6):
            data = _encode(frames, max(10, attempt), 6, frame_ms)
            if _b64_len(len(data)) <= room:
                return base64.b64encode(data).decode(), width, max(10, attempt)
    return None
