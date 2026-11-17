"""GlowTube: plays YouTube videos and streams them into a T3 Code theme.

T3 Code themes are CSS only, cannot fetch anything, and reload when their file changes, after
100 ms of quiet. So every BATCH seconds (a quarter second) this helper rewrites the theme with:
  * an animated AVIF of the next 8 video frames at up to the source's 640x360, which the browser
    plays by itself at 24 fps;
  * keyframes for a 16-band spectrum, bass, kicks and four edge colors.
Each write also carries an animated AVIF of the light those frames throw, pre-rendered here
(blurred, widened by the bass, carrying the kick shockwave), and the Now Playing card image.
T3 restarts every CSS transition and animation when it applies a write, so nothing can be held
over from an earlier write; every write is complete on its own.
The audio plays here, and its clock decides which frames go into each batch. Any control
command rewrites the theme at once instead of waiting for the next batch.

The Now Playing card is clickable: play/pause, scrub, pick a queued track to play next, and play or queue
the YouTube link on the clipboard. The keyboard media keys play, pause and skip too.

  python glowtube.py play [<url or file> ...]
  python glowtube.py ctl toggle|next|prev|seek 10|vol 0.7|chaos 11|offset 0.05|add <url>|status|quit
  python glowtube.py idle                       write the idle theme and exit
"""

from __future__ import annotations

import argparse
import base64
import colorsys
import ctypes
import io
import json
import math
import os
import re
import subprocess
import sys
import threading
import time
import urllib.parse
import urllib.request
from ctypes import wintypes
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import numpy as np
import sounddevice as sd
from PIL import Image, ImageFilter

import card_image
import encoder
import t3api
from concurrent.futures import Future, ProcessPoolExecutor
import theme_css

HERE = Path(__file__).resolve().parent
CACHE = HERE / "cache"
THEMES_DIR = Path(os.environ.get("T3_THEMES_DIR", Path.home() / ".t3" / "userdata" / "themes"))
THEME_ID = "glowtube"
CTL_PORT = int(os.environ.get("GLOWTUBE_PORT", "47873"))

FPS = 24                             # matches most music videos (23.976); 30 would repeat every 4th frame
SR = 48000
FRAME_W, FRAME_H = 640, 360          # decode size: the cached videos' native resolution
VIDEO_W, VIDEO_H = 800, 450          # size on screen
GLOW_W, GLOW_H = 128, 72             # glow image; the player covers the middle quarter of it
BATCH = 0.25                         # seconds between theme writes; must stay above T3's 100 ms debounce
STEP = math.ceil(BATCH * FPS)        # frames between write starts: writes land on a 6-frame grid
SPAN = STEP + 2                      # video frames per write, with spares in case the next write is late
MIN_GAP = 0.15                       # quiet time T3 needs after a write before it reloads
QUIET_SECONDS = 2.5                  # length of the long write that covers a favicon swap
QUIET_HOLD = 2.2                     # how long writes pause after it
QUIET_SETTLE = 0.45                  # wait after the long write so T3 has applied it before the swap
CSS_LIMIT = 49_500                   # T3 allows 50,000 characters

YOUTUBE = re.compile(
    r"https?://(?:www\.|m\.|music\.)?(?:youtube\.com/(?:watch\?|shorts/|live/|playlist\?)|youtu\.be/)[^\s\"'<>]+"
)


# ---------------------------------------------------------------- tracks


@dataclass
class Track:
    url: str
    id: str = ""
    title: str = "Loading…"
    artist: str = ""
    path: Path | None = None
    pcm: np.ndarray | None = None      # int16 stereo
    features: dict = field(default_factory=dict)
    frames: int = 0
    failed: bool = False
    loading: bool = False


def run(cmd: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=True, capture_output=True)


def expand(url: str) -> list[tuple[str, str]]:
    """The videos behind a link, with titles: one for a video, all of them for a playlist."""
    if Path(url).is_file():
        return [(url, Path(url).stem)]
    info = json.loads(run(["yt-dlp", "--js-runtimes", "node", "-J", "--flat-playlist", url]).stdout)
    if info.get("_type") == "playlist":
        return [(e.get("url") or f"https://www.youtube.com/watch?v={e['id']}", e.get("title") or "")
                for e in info.get("entries", []) if e]
    return [(url, info.get("track") or info.get("title") or "")]


def load(track: Track) -> None:
    local = Path(track.url)
    if track.path is not None and track.path.exists():
        path = track.path                  # loaded before and unloaded to save memory
    elif local.is_file():
        track.id, track.title, track.artist, path = local.stem, local.stem, "local file", local.resolve()
    else:
        info = json.loads(run(["yt-dlp", "--js-runtimes", "node", "-J", "--no-playlist", track.url]).stdout)
        track.id = info["id"]
        track.title = info.get("track") or info.get("title") or track.id
        track.artist = info.get("artist") or info.get("uploader") or info.get("channel") or ""
        CACHE.mkdir(exist_ok=True)
        path = CACHE / f"{track.id}.mp4"
        if not path.exists():
            print(f"downloading {track.title}", flush=True)
            run([
                "yt-dlp", "--js-runtimes", "node", "--no-playlist", "-q", "--merge-output-format", "mp4",
                "-f", "bv*[height<=360]+ba/b[height<=360]/b",
                "-o", str(CACHE / "%(id)s.%(ext)s"), track.url,
            ])
    track.path = path
    raw = run(["ffmpeg", "-v", "error", "-i", str(path), "-f", "s16le", "-ac", "2", "-ar", str(SR), "-"]).stdout
    pcm = np.frombuffer(raw, dtype=np.int16).reshape(-1, 2)
    track.features = analyse(pcm)
    track.frames = int(len(pcm) / SR * FPS)
    track.pcm = pcm                     # last, because the player treats it as "ready"
    print(f"ready: {track.title} ({len(pcm) / SR:.0f}s)", flush=True)


# ---------------------------------------------------------------- audio analysis


def analyse(pcm: np.ndarray) -> dict:
    """Per video frame: 16 log-spaced bands, bass, bass kicks and shockwave radius."""
    mono = pcm.astype(np.float32).mean(axis=1) / 32768.0
    n = int(len(mono) / SR * FPS)
    win = 4096                          # long enough to resolve 30-150 Hz
    hop = SR / FPS
    pad = np.pad(mono, (win, win))
    window = np.hanning(win).astype(np.float32)
    freqs = np.fft.rfftfreq(win, 1 / SR)
    nb = theme_css.BANDS
    edges = np.geomspace(40, 14000, nb + 1)
    band_idx = [np.where((freqs >= edges[i]) & (freqs < edges[i + 1]))[0] for i in range(nb)]
    bass_idx = np.where((freqs >= 28) & (freqs < 150))[0]

    bands = np.zeros((n, nb), np.float32)
    bass = np.zeros(n, np.float32)
    for start in range(0, n, 512):
        ks = np.arange(start, min(n, start + 512))
        idx = (ks[:, None] * hop).astype(np.int64) + win // 2 + np.arange(win)[None, :]
        mag = np.abs(np.fft.rfft(pad[idx] * window, axis=1))
        for b, sel in enumerate(band_idx):
            bands[ks, b] = np.log1p(mag[:, sel].mean(axis=1) * 8)
        bass[ks] = np.log1p(mag[:, bass_idx].mean(axis=1) * 8)

    def normalise(x, lo_pct=8, hi_pct=98.5):
        lo, hi = np.percentile(x, lo_pct), np.percentile(x, hi_pct)
        return np.clip((x - lo) / max(hi - lo, 1e-6), 0, 1)

    def envelope(x, release):
        out = np.empty_like(x)
        y = 0.0
        for i, v in enumerate(x):
            y = v if v > y else y * release + v * (1 - release)
            out[i] = y
        return out

    bands = np.stack([envelope(normalise(bands[:, b]), 0.8) for b in range(nb)], axis=1)
    # Only real bass hits should push hard: a high floor, then a curve that keeps quiet bass small.
    bass = envelope(normalise(bass, 30, 99.5) ** 1.4, 0.72)

    flux = np.maximum(np.diff(bass, prepend=bass[0]), 0)
    kick = np.zeros(n, np.float32)
    ring = np.zeros(n, np.float32)
    k, since, last = 0.0, 0.0, -99
    for i in range(n):
        lo, hi = max(0, i - 45), min(n, i + 45)
        threshold = flux[lo:hi].mean() + 1.5 * flux[lo:hi].std() + 0.03
        if flux[i] > threshold and bass[i] > 0.25 and i - last > 6:
            k, last, since = 1.0, i, 0
        else:
            k *= 0.82
            since += 1
        kick[i] = k
        ring[i] = since * 1440 / FPS    # screen pixels the shockwave has travelled (1440 px/s)
    return {"bands": bands, "bass": bass, "kick": kick, "ring": ring}


# ---------------------------------------------------------------- video


class VideoReader:
    """Decodes small RGB frames ahead of the play position and restarts on seeks and loops."""

    def __init__(self, path: Path):
        self.path = path
        self.frames: dict[int, np.ndarray] = {}
        self.lock = threading.Condition()
        self.proc: subprocess.Popen | None = None
        self.start_at(0)

    def start_at(self, frame: int) -> None:
        self.stop()
        with self.lock:
            self.frames.clear()
            self.base = self.decoded = self.wanted = frame
        self.proc = subprocess.Popen(
            ["ffmpeg", "-v", "error", "-ss", f"{frame / FPS:.3f}", "-i", str(self.path), "-an",
             "-vf", f"fps={FPS},scale={FRAME_W}:{FRAME_H}:force_original_aspect_ratio=increase:flags=area,crop={FRAME_W}:{FRAME_H}", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        )
        threading.Thread(target=self._pump, args=(self.proc,), daemon=True).start()

    def _pump(self, proc: subprocess.Popen) -> None:
        size = FRAME_W * FRAME_H * 3
        while True:
            buf = proc.stdout.read(size)
            if not buf or len(buf) < size:
                return
            with self.lock:
                while self.decoded - self.wanted > FPS * 2 and proc is self.proc:
                    self.lock.wait(0.2)
                if proc is not self.proc:
                    return
                self.frames[self.decoded] = np.frombuffer(buf, np.uint8).reshape(FRAME_H, FRAME_W, 3)
                self.decoded += 1
                # a write asks for a whole second of frames, so keep a few seconds behind the last one
                for old in [f for f in self.frames if f < self.wanted - FPS * 2]:
                    del self.frames[old]

    def get(self, frame: int) -> np.ndarray | None:
        with self.lock:
            self.wanted = frame
            self.lock.notify_all()
            hit = self.frames.get(frame)
            if hit is not None:
                return hit
            oldest = min(self.frames, default=self.base)
            restart = frame < oldest or frame > self.decoded + FPS * 2
        if restart:
            self.start_at(max(0, frame))
            return None
        with self.lock:
            older = [f for f in self.frames if f <= frame]
            return self.frames[max(older)] if older else None

    def stop(self) -> None:
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
        self.proc = None


def light(rgb) -> str:
    """A sampled color as light: same hue, a little more saturated, never fully dark."""
    r, g, b = (float(c) / 255 for c in rgb)
    h, s, v = colorsys.rgb_to_hsv(r, g, b)
    s = min(1.0, s * 1.2)
    v = min(1.0, v * 1.1 + 0.08)
    r, g, b = colorsys.hsv_to_rgb(h, s, v)
    return f"#{int(r * 255):02x}{int(g * 255):02x}{int(b * 255):02x}"


def edge_colors(frame: np.ndarray) -> list[str]:
    """Top, right, bottom and left edge colors, then the average of the whole frame."""
    h, w, _ = frame.shape
    sy, sx = h // 5, w // 6
    strips = [frame[:sy], frame[:, w - sx:], frame[h - sy:], frame[:, :sx], frame]
    return [light(s.reshape(-1, 3).mean(axis=0)) for s in strips]


_GY, _GX = np.mgrid[0:GLOW_H, 0:GLOW_W].astype(np.float32)
_GDIST = np.hypot(_GX - (GLOW_W - 1) / 2, _GY - (GLOW_H - 1) / 2)
GLOW_PX = VIDEO_W * theme_css.GLOW_SCALE / GLOW_W     # screen pixels per glow pixel


def glow_frame(frame: np.ndarray, bass: float, kick: float, ring: float) -> Image.Image:
    """The light a TV backlight kit would throw for this frame, drawn at low resolution.

    The frame is shrunk into the middle quarter of a dark canvas and blurred outwards. Bass widens
    and brightens the blur; a kick adds a flash and a ring that travels out from the player."""
    # Painted larger than the player so the light spills past its edges, like an LED strip
    # behind a TV; bass and kicks push it further out.
    spill = 1.35 + bass * 1.3 + kick * 0.5
    small = Image.fromarray(frame).resize((round(GLOW_W / theme_css.GLOW_SCALE * spill),
                                           round(GLOW_H / theme_css.GLOW_SCALE * spill)), Image.BOX)
    canvas = Image.new("RGB", (GLOW_W, GLOW_H))
    canvas.paste(small, ((GLOW_W - small.width) // 2, (GLOW_H - small.height) // 2))
    radius = 2.5 + bass * 5.0 + kick * 2.0
    glow = np.asarray(canvas.filter(ImageFilter.GaussianBlur(radius)), np.float32)
    glow *= 1.8 + bass * 3.5 + kick * 3.0
    if kick > 0.05:
        avg = np.asarray(frame, np.float32).reshape(-1, 3).mean(axis=0)
        avg = avg / max(avg.max(), 1) * 255
        shell = np.exp(-(((_GDIST - ring / GLOW_PX) / 1.3) ** 2)) * kick
        glow += shell[..., None] * avg[None, None, :]
    # more saturated, then dimmed by the brightest channel instead of clipped, so hues survive
    lum = glow.mean(axis=2, keepdims=True)
    glow = np.maximum(lum + (glow - lum) * 1.5, 0)
    glow *= np.minimum(1.0, 255 / np.maximum(glow.max(axis=2, keepdims=True), 1e-3))
    return Image.fromarray(glow.astype(np.uint8))


def avif(frames: list[Image.Image], quality: int) -> str:
    buf = io.BytesIO()
    frames[0].save(buf, "AVIF", save_all=len(frames) > 1, append_images=frames[1:], duration=round(1000 / FPS),
                   loop=1, quality=quality, speed=10, max_threads=4)
    return base64.b64encode(buf.getvalue()).decode()


# ---------------------------------------------------------------- player


class Player:
    def __init__(self, urls: list[str]):
        self.queue: list[Track] = []
        self.index = 0
        self.lock = threading.RLock()
        self.playing = True
        self.pos = 0                      # next sample to hand to the sound card
        self.volume = float(os.environ.get("GLOWTUBE_VOLUME", "0.8"))
        self.chaos = 11.0
        self.offset = 0.16                # seconds from writing a theme to it showing on screen
        self.cb_wall = time.perf_counter()
        self.cb_pos = 0
        self.running = True
        self.reader: VideoReader | None = None
        self.status, self.status_until = "", 0.0
        self.wake = threading.Event()     # set by commands so the theme is rewritten at once
        self.build_est = 0.15             # seconds a batch takes to build, measured as it runs
        self.next_k = None                # grid frame the next write should start at
        self.hold_writes_until = 0.0      # no theme writes before this (monotonic seconds)
        self.favicon_enabled = True       # ctl "favicon off" stops favicon video (the theme video plays)
        self.favicon_live = False         # a favicon clip covers the player, so the theme video is hidden
        self.favicon_front: str | None = None   # which favicon deck is on top ("a" or "b")
        # T3 throws away a favicon load if a theme update lands while it runs, so a swap only completes
        # when the theme is quiet for a moment. The favicon video asks for a quiet window; the write loop
        # answers with one long write (QUIET_SECONDS of content) and then holds further writes.
        self.quiet_requested = False
        self.quiet_granted = 0.0
        self.batch_quiet = False          # the batch being built is the long one for a quiet window
        self._card_key, self._card = None, None
        for url in urls:
            self.add(url, "end")
        threading.Thread(target=self._loader, daemon=True).start()
        self.stream = sd.OutputStream(samplerate=SR, channels=2, dtype="int16", callback=self._callback,
                                      latency="low")
        self.stream.start()

    def say(self, message: str, seconds: float = 4.0) -> None:
        """Show a short message in the card's subtitle line."""
        self.status, self.status_until = message, time.monotonic() + seconds
        self.wake.set()

    # -- queue
    def add(self, url: str, mode: str = "end") -> None:
        """Queue a link. mode: "now" plays it at once, "next" puts it next, "end" appends it."""
        def work():
            self.say("Fetching the link…", 60)
            try:
                items = expand(url)
            except Exception as exc:
                self.say(f"Could not open that link ({type(exc).__name__})")
                return
            tracks = [Track(u, title=title or "Loading…") for u, title in items]
            with self.lock:
                if not self.queue:
                    self.queue, self.index, self.pos = tracks, 0, 0
                elif mode in ("now", "next"):
                    self.queue[self.index + 1:self.index + 1] = tracks
                    if mode == "now":
                        self.index, self.pos, self.playing = self.index + 1, 0, True
                else:
                    self.queue.extend(tracks)
            what = f"{len(tracks)} tracks" if len(tracks) > 1 else (tracks[0].title if tracks else "nothing")
            self.say({"now": "Playing ", "next": "Up next: ", "end": "Queued: "}[mode] + what)
        threading.Thread(target=work, daemon=True).start()

    def play_next(self, offset: int) -> None:
        """Move the queued track `offset` places after the current one to play next."""
        with self.lock:
            src = self.index + 1 + offset
            if offset > 0 and src < len(self.queue):
                self.queue.insert(self.index + 1, self.queue.pop(src))
                self.say(f"Up next: {self.queue[self.index + 1].title}")

    def upcoming(self, count: int) -> list[Track]:
        with self.lock:
            return self.queue[self.index + 1:self.index + 1 + count]

    def _loader(self) -> None:
        """Keeps the current and next tracks downloaded and decoded, and frees the rest, so a long
        playlist neither downloads everything at once nor holds every track's audio in memory."""
        while self.running:
            with self.lock:
                wanted = [t for t in (self.track, *self.upcoming(1)) if t is not None]
                for t in self.queue:
                    if t.pcm is not None and all(t is not w for w in wanted):
                        t.pcm, t.features = None, {}
            for t in wanted:
                if t.pcm is None and not t.failed:
                    t.loading = True
                    self.wake.set()
                    try:
                        load(t)
                    except Exception as exc:
                        t.failed = True
                        print(f"could not load {t.url}: {exc}", file=sys.stderr, flush=True)
                    finally:
                        t.loading = False
                        self.wake.set()
            time.sleep(0.3)

    def card(self, subtitle: str) -> str:
        """CSS for the Now Playing card, re-drawn only when what it shows has changed."""
        t = self.track
        if time.monotonic() < self.status_until:
            subtitle = self.status
        title = t.title if t else "Nothing playing"
        items = [card_image.QueueItem(q.title, "loading" if q.loading else "failed" if q.failed else "")
                 for q in self.upcoming(card_image.QUEUE_ROWS)]
        key = (title, subtitle, self.playing, tuple((i.title, i.state) for i in items))
        if key != self._card_key:
            card = card_image.render(title, subtitle, self.playing, items)
            self._card_key = key
            self._card = theme_css.card_height_css(card.height) + theme_css.card_image_css(card)
        return self._card

    @property
    def track(self) -> Track | None:
        with self.lock:
            return self.queue[self.index] if 0 <= self.index < len(self.queue) else None

    def ready(self) -> bool:
        t = self.track
        return bool(t and t.pcm is not None)

    def jump(self, delta: int) -> None:
        with self.lock:
            if self.queue:
                self.index = (self.index + delta) % len(self.queue)
                self.pos = 0

    def seek_to(self, fraction: float) -> None:
        with self.lock:
            t = self.track
            if t and t.pcm is not None:
                self.pos = int(np.clip(fraction, 0, 1) * (len(t.pcm) - 1))

    def seek(self, seconds: float) -> None:
        with self.lock:
            t = self.track
            if t and t.pcm is not None:
                self.pos = int(np.clip(self.pos + seconds * SR, 0, len(t.pcm) - 1))

    # -- audio
    def _callback(self, out, frames, time_info, status):
        with self.lock:
            t = self.track
            if not (self.playing and t is not None and t.pcm is not None):
                out.fill(0)
                self.cb_pos, self.cb_wall = self.pos, time.perf_counter()
                if t is not None and t.failed and len(self.queue) > 1:
                    self.index = (self.index + 1) % len(self.queue)
                return
            chunk = t.pcm[self.pos:self.pos + frames]
            out[: len(chunk)] = (chunk * self.volume).astype(np.int16)
            out[len(chunk):] = 0
            self.cb_pos, self.cb_wall = self.pos, time.perf_counter()
            self.pos += frames
            if self.pos >= len(t.pcm):
                self.index = (self.index + 1) % len(self.queue)
                self.pos = 0

    def media_time(self) -> float:
        """Position of the sample leaving the speakers right now."""
        with self.lock:
            if not self.playing:
                return self.pos / SR
            return max(0.0, self.cb_pos / SR + (time.perf_counter() - self.cb_wall) - self.stream.latency)

    # -- frames for the theme
    def batch(self) -> str | None:
        t = self.track
        if not self.ready():
            return None
        if self.reader is None or self.reader.path != t.path:
            if self.reader:
                self.reader.stop()
            self.reader = VideoReader(t.path)
        if self.playing:
            start, count = self.media_time() + self.build_est + self.offset, SPAN
            self.batch_quiet = self.quiet_requested   # only this long write may open the quiet window
            if self.batch_quiet:
                count = round(QUIET_SECONDS * FPS)
        else:
            start, count = self.pos / SR, 1
        k0 = min(max(0, int(round(start * FPS))), t.frames - 1)
        if self.playing and self.next_k is not None and abs(k0 - self.next_k) <= 2:
            k0 = self.next_k              # on schedule: stay on the grid the clips were encoded for
        # hidden under the favicon clip: spend nothing on it. The long quiet write is cheap too: 2.5 s of
        # video in one write can't be sharp anyway, and it must be built quickly for the swap.
        cheap = self.favicon_live or (self.playing and self.batch_quiet)
        prepared = AHEAD.take(t.id, k0) if self.playing and not cheap else None
        if prepared:
            k0 = prepared[0]             # within a frame of the ideal start; use the clip's own start
        end = min(k0 + count, t.frames)
        f = t.features
        frames, glows, values, last = [], [], [], None
        for k in range(k0, end):
            img = self.reader.get(k)
            img = img if img is not None else last
            if img is None:
                continue
            last = img
            glows.append(glow_frame(img, float(f["bass"][k]), float(f["kick"][k]), float(f["ring"][k])))
            frames.append(Image.fromarray(img))
            # The browser blends between keyframes, so slow values skip frames: colors and progress every 6th,
            # the spectrum every 2nd. Bass and kicks go in every frame so hits stay sharp.
            i = k - k0
            final = k == end - 1
            v = {"bs": f"{f['bass'][k]:.2f}", "kk": f"{f['kick'][k]:.2f}"}
            if i % 6 == 0 or final:
                v["pr"] = f"{k / max(1, t.frames - 1):.4f}"
                colors = edge_colors(img)
                v.update({f"c{j}": c for j, c in enumerate(colors[:4])}, aa=colors[4])
            if i % 2 == 0 or final:
                v.update({f"b{b}": f"{f['bands'][k, b]:.2f}" for b in range(theme_css.BANDS)})
            values.append(v)
        if not frames:
            return None
        elapsed, total = k0 / FPS, t.frames / FPS
        clock = f"{int(elapsed // 60)}:{int(elapsed % 60):02d} / {int(total // 60)}:{int(total % 60):02d}"
        card = self.card(f"{t.artist}  ·  {clock}" if t.artist else clock)
        if self.favicon_front:
            card += theme_css.favicon_front_css(self.favicon_front)
        if MEASURE:                       # tools/overlay_chart.py: any magenta on screen is a gap in the favicon video
            card += theme_css.CONV + "::after{background:#ff00ff!important}"
        css = build_dynamic(frames, glows, values, card, self.playing, self.chaos, prepared, cheap)
        if self.playing and count > SPAN:
            self.next_k = None            # a long write for a favicon swap; the grid picks up after it
        elif self.playing:
            self.next_k = k0 + STEP
            for n in range(1, AHEAD_WRITES + 1):
                if not cheap:
                    self._queue_ahead(t, k0 + n * STEP)
        else:
            self.next_k = None
            AHEAD.clear()
        return css

    def _queue_ahead(self, t: Track, k: int) -> None:
        """Start encoding the clip that starts at grid frame k, at the slow, better setting."""
        if k >= t.frames:
            return
        end = min(k + SPAN, t.frames)
        raw = []
        for j in range(k, end):
            img = self.reader.get(j)
            if img is None:
                return                    # not decoded yet; that write falls back to the fast encoder
            raw.append(img.tobytes())
        AHEAD.submit(t.id, k, b"".join(raw), end - k)

    def until_next(self) -> float:
        """Seconds until the audio reaches the next grid frame, so the write lands on schedule."""
        if not self.playing or self.next_k is None:
            return BATCH
        ideal = (self.media_time() + self.build_est + self.offset) * FPS
        return min(BATCH * 2, max(0.0, (self.next_k - ideal) / FPS))


SEQ = 0
# Ways to encode a second of video, best first: (width, keep every Nth frame, AVIF quality).
# Busy, fast-cut footage needs several times the bytes of a calm shot, so the helper walks down
# this list until the write fits, remembers where it landed, and tries one step better next time.
LADDER = (
    [(640, 1, q) for q in (60, 52, 45, 38, 32, 26, 21)]
    + [(512, 1, q) for q in (24, 18)]
    + [(384, 1, q) for q in (20, 14)]
    + [(320, 1, 10)]
)
LEVEL = 4
QUALITY = "starting"


def encode_video(frames: list[Image.Image], level: int) -> str:
    width, step, quality = LADDER[level]
    picked = frames[::step]
    if width != frames[0].width:
        size = (width, round(width * 9 / 16))
        picked = [f.resize(size, Image.BILINEAR) for f in picked]
    buf = io.BytesIO()
    picked[0].save(buf, "AVIF", save_all=len(picked) > 1, append_images=picked[1:],
                   duration=round(step * 1000 / FPS), loop=1, quality=quality, speed=10, max_threads=4)
    return base64.b64encode(buf.getvalue()).decode()


def build_dynamic(frames, glows, values, card: str, playing: bool, chaos: float,
                  prepared: tuple[int, str, int, int] | None = None, cheap: bool = False) -> str:
    global SEQ, LEVEL, QUALITY
    SEQ = (SEQ + 1) % 100000
    n = len(frames)
    if playing and n > 1:
        vars_css = (theme_css.keyframe_block(f"d{SEQ}", values)
                    + theme_css.animate(f"d{SEQ}", n / FPS))
    else:
        vars_css = theme_css.static_values(values[0])
    head = f":root{{--g:{chaos / 11:.2f}}}{vars_css}" + card
    if glows:
        head += theme_css.glow_css(avif(glows, 30))
    room = CSS_LIMIT - len(STATIC) - len(head) - len(':root{--vid:url("data:image/avif;base64,")}')
    AHEAD.room = room
    if prepared is not None:
        _, data, width, quality = prepared
        fits = len(data) <= room
        AHEAD.stats["used" if fits else "too_big"] += 1
        if fits:
            QUALITY = f"{width}x{round(width * 9 / 16)} 24fps q{quality} (ahead, speed 6)"
            return STATIC + head + f':root{{--vid:url("data:image/avif;base64,{data}")}}'
    # No usable clip from the workers (start of play, seek, pause, or it came out too big):
    # encode now at the fast setting. Start where the last write landed and step down only if this one doesn't fit, so most writes
    # encode the video once. How full the write ended up decides where the next one starts.
    level = len(LADDER) - 3 if cheap else LEVEL
    while True:
        data = encode_video(frames, level)
        if len(data) <= room or level == len(LADDER) - 1:
            break
        level += 1
    if len(data) > room:               # nothing fits: hold a single still frame for this write
        data = encode_video(frames[:1], len(LADDER) - 1)
    fill = len(data) / room
    if not cheap:
        LEVEL = max(0, level - (2 if fill < 0.55 else 1 if fill < 0.8 else 0))
    width, step, quality = LADDER[level]
    QUALITY = f"{width}x{round(width * 9 / 16)} {FPS // step}fps q{quality} (now, speed 10)"
    return STATIC + head + f':root{{--vid:url("data:image/avif;base64,{data}")}}'


STATIC = theme_css.static_css(VIDEO_W, VIDEO_H)

AHEAD_WRITES = 5                     # clips are encoded this many writes before they are needed


class AheadEncoder:
    """Encodes upcoming clips in worker processes at libavif speed 6, about 2 dB better than the
    speed 10 used for immediate encodes, but too slow (200-400 ms) to run once per 250 ms write.

    Each clip is keyed by track and start frame. The worker sizes it to the room the last write
    had (see encoder.encode_to_fit); if it still doesn't fit when its write comes, that write falls
    back to a fast encode."""

    def __init__(self) -> None:
        self.pool: ProcessPoolExecutor | None = None
        self.jobs: dict[tuple[str, int], Future] = {}
        self.room = 30000
        self.stats = {"used": 0, "too_big": 0, "not_ready": 0, "none_queued": 0}

    def start(self, workers: int = 6) -> None:
        self.pool = ProcessPoolExecutor(max_workers=workers)

    def submit(self, track_id: str, k: int, raw: bytes, count: int) -> None:
        if self.pool is None or (track_id, k) in self.jobs:
            return
        room = self.room - 600            # leave slack: the card and glow vary a little per write
        self.jobs[(track_id, k)] = self.pool.submit(encoder.encode_to_fit, raw, (FRAME_W, FRAME_H), count, room,
                                                    round(1000 / FPS))

    def take(self, track_id: str, k0: int) -> tuple[int, str, int, int] | None:
        """The finished clip starting within one frame of k0, as (start, base64, width, quality)."""
        found = None
        for (tid, k), fut in list(self.jobs.items()):
            if tid != track_id or k < k0 - 1:
                fut.cancel()
                del self.jobs[(tid, k)]
            elif found is None and abs(k - k0) <= 1 and fut.done():
                del self.jobs[(tid, k)]
                try:
                    result = None if fut.cancelled() else fut.result()
                except Exception as exc:
                    print(f"ahead encode failed: {exc!r}", file=sys.stderr, flush=True)
                    result = None
                if result:
                    found = (k, *result)
                else:
                    self.stats["too_big"] += 1
        if found is None:
            pending = any(t == track_id and abs(k - k0) <= 1 for t, k in self.jobs)
            self.stats["not_ready" if pending else "none_queued"] += 1
        return found

    def clear(self) -> None:
        for fut in self.jobs.values():
            fut.cancel()
        self.jobs.clear()

    def stop(self) -> None:
        self.clear()
        if self.pool:
            self.pool.shutdown(wait=False, cancel_futures=True)


AHEAD = AheadEncoder()


# ---------------------------------------------------------------- favicon video

CLIPS = HERE / "clips"
MEASURE = bool(os.environ.get("GLOWTUBE_MEASURE"))
CLIP_SECONDS = 45.0                  # longest favicon clip
FIRST_CLIP_SECONDS = 6.0             # after a start or seek: short, encoded at the fast setting
FIRST_LEAD = 3.0                     # seconds from queuing that first clip to its first frame (after a start
                                     # or seek, the first long quiet write takes ~1.6 s to build while frames load)
HANDOVER = 3.0                       # a clip on the back deck starts this long before the front one ends
FLIP_AFTER = 1.2                     # seconds after publishing a back-deck clip before it is brought forward
SHOW_LATENCY = float(os.environ.get("GLOWTUBE_FAVICON_LATENCY", "0.5"))  # API call to first frame (measured 0.45 s)
QUIET_BUILD = 0.8                    # request to long write landing (measured 0.5-0.65 s, see "[quiet]" log lines)
PUBLISH_LEAD = SHOW_LATENCY + 0.85   # an encode must be done this long before its clip starts (quiet window)
# Encode time per second of clip, measured on this machine with the helper running (tools/ timing):
# speed 10 about 0.15 s, speed 8 about 0.22 s, plus about 0.25 s to start ffmpeg. 1.3x margin.
ENCODE_COST = {True: 0.15 * 1.3, False: 0.22 * 1.3}
ENCODE_START = 0.3
BLANK = '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"/>'


@dataclass
class Deck:
    """One project whose favicon carries clips: "a" is the GlowTube project, "b" is GlowTube B."""
    name: str
    project_id: str
    clip: tuple[str, float, float] | None = None   # (track id, start, end) of the clip it shows
    published_at: float = 0.0                        # monotonic time of the last publish
    blank: bool = False


@dataclass
class Planned:
    """A clip being encoded or waiting to be published."""
    fut: Future
    track: str
    start: float
    end: float
    deck: Deck
    done_by: float                   # media time its encode should be finished


def encode_seconds(seconds: float, fast: bool) -> float:
    return ENCODE_START + seconds * ENCODE_COST[fast]


class FaviconVideo:
    """Full-quality video through project favicons.

    T3 shows a project's favicon with a real <img>, and the theme moves that image over the player.
    Each clip is an animated AVIF wrapped in an SVG file of over 4 MiB (T3's icon loader skips files
    that big at once instead of downloading them to try caching them); T3 shows a new one about
    0.45 s after the faviconPath changes, provided no theme write lands meanwhile.

    Swapping a favicon removes the old image while the new one loads, so with two projects ("decks")
    the swap happens out of sight: while the front deck's clip plays on top, the next clip goes to
    the back deck, which starts it in step with the audio underneath; a theme write then brings the
    back deck to the front. With one deck, each swap shows the theme's own video for a moment.

    Clips are planned back to back: each next clip is queued as soon as the one before it is queued,
    and is as long as the encoder can finish before it is due (so after a start or seek they grow
    from 6 s to 45 s)."""

    def __init__(self, player: "Player", api: t3api.T3Api, decks: list[Deck]):
        self.player, self.api, self.decks = player, api, decks
        self.front: Deck | None = None
        self.plan: list[Planned] = []
        self.seq = 0
        self.run_id = time.strftime("%H%M%S")   # clip paths stay unique across restarts
        self.pool = ProcessPoolExecutor(max_workers=1)   # never queued behind the theme's short clips
        self.pool.submit(encoder.warm_up)  # start the worker (and import its libraries) before the first clip is due
        CLIPS.mkdir(exist_ok=True)
        (CLIPS / "glowtube-blank.svg").write_text(BLANK)
        threading.Thread(target=self._loop, daemon=True).start()

    # -- T3
    def _publish(self, deck: Deck, path: Path) -> None:
        try:
            self.api.update_project(deck.project_id, faviconPath=str(path))
        except t3api.T3Error as exc:
            print(f"favicon update failed: {exc}", file=sys.stderr, flush=True)

    def _drop_plan(self) -> None:
        for planned in self.plan:
            planned.fut.cancel()
        self.plan = []

    def _blank_all(self) -> None:
        self._set_front(None)
        self._drop_plan()
        for deck in self.decks:
            deck.clip = None
            if not deck.blank:
                deck.blank = True
                self._publish(deck, CLIPS / "glowtube-blank.svg")

    def _set_front(self, deck: Deck | None) -> None:
        self.front = deck
        self.player.favicon_live = deck is not None
        self.player.favicon_front = deck.name if deck else None
        self.player.wake.set()

    @staticmethod
    def _log(msg: str) -> None:
        if os.environ.get("GLOWTUBE_DEBUG"):
            print(f"[favicon {time.strftime('%H:%M:%S')}] {msg}", flush=True)

    def _cleanup(self, keep: int = 30) -> None:
        clips = sorted(CLIPS.glob("glowtube-clip-*.svg"), key=lambda p: p.stat().st_mtime)
        for old in clips[:-keep]:
            old.unlink(missing_ok=True)

    def _loop(self) -> None:
        while self.player.running:
            time.sleep(0.1)
            try:
                self._tick()
            except Exception as exc:
                print(f"favicon video: {exc!r}", file=sys.stderr, flush=True)

    def _other(self, deck: Deck | None) -> Deck:
        if deck is None or len(self.decks) == 1:
            return self.decks[0]
        return self.decks[1] if deck is self.decks[0] else self.decks[0]

    # -- planning
    def _queue(self, t: Track, deck: Deck, start: float, seconds: float, fast: bool, ready_at: float) -> None:
        seconds = min(seconds, t.frames / FPS - start)
        if seconds < 1:
            return                        # the track is about to end
        self.seq += 1
        out = CLIPS / f"glowtube-clip-{deck.name}-{self.run_id}-{self.seq}.svg"
        speed, quality = (10, 55) if fast else (8, 60)
        fut = self.pool.submit(encoder.encode_clip_svg, str(t.path), start, seconds, str(out), FPS, (FRAME_W, FRAME_H),
                               quality, speed)
        done_by = ready_at + encode_seconds(seconds, fast)
        self.plan.append(Planned(fut, t.id, start, start + seconds, deck, done_by))
        self._log(f"queued {start:.1f}-{start + seconds:.1f}s for deck {deck.name} "
                  f"({'fast' if fast else 'good'}, due done at {done_by:.1f}s)")

    def _plan_next(self, t: Track, now: float) -> None:
        if len(self.plan) >= 2:
            return
        if self.plan:
            last = self.plan[-1]
            prev_start, prev_end, prev_deck, ready_at = last.start, last.end, last.deck, max(now, last.done_by)
        elif self.front and self.front.clip:
            # the newest published clip, which may still be waiting behind the front one
            prev_deck = max((d for d in self.decks if d.clip and d.clip[0] == t.id), key=lambda d: d.clip[1])
            _, prev_start, prev_end = prev_deck.clip
            ready_at = now
        else:                             # nothing playing: a short clip encoded fast, as soon as possible
            self._queue(t, self.decks[0], now + FIRST_LEAD, FIRST_CLIP_SECONDS, True, now)
            return
        if prev_end >= t.frames / FPS - 0.5:
            return
        handover = min(HANDOVER, 0.4 * (prev_end - prev_start)) if len(self.decks) > 1 else 1.0
        # if the encoder is behind (say after a dropped clip), start later: a short gap beats none at all
        start = max(prev_end - handover, ready_at + PUBLISH_LEAD + encode_seconds(handover + 3.0, True))
        budget = start - PUBLISH_LEAD - ready_at
        good = (budget - ENCODE_START) / ENCODE_COST[False]
        fast = good < 10
        seconds = (budget - ENCODE_START) / ENCODE_COST[fast]
        seconds = max(handover + 3.0, min(CLIP_SECONDS + handover, seconds))
        self._queue(t, self._other(prev_deck), start, seconds, fast, ready_at)

    # -- scheduling
    def _tick(self) -> None:
        p = self.player
        t = p.track
        if not (t and t.pcm is not None and t.path and p.playing and p.favicon_enabled):
            if self.front or self.plan or not all(d.blank for d in self.decks):
                self._blank_all()
            return
        now = p.media_time()
        front = self.front
        if front and not (front.clip and front.clip[0] == t.id and front.clip[1] - 0.6 <= now < front.clip[2]):
            self._log(f"front clip no longer matches (now {now:.1f}s); starting over")
            self._blank_all()             # seek, track change, or the clip ran out
            front = None
        if self.plan and (self.plan[0].track != t.id or now > self.plan[0].start + 0.6):
            first = self.plan[0]
            self._log(f"dropped clip for {first.start:.1f}s (now {now:.1f}s, done={first.fut.done()}, quiet "
                      f"requested={p.quiet_requested}, granted {time.monotonic() - p.quiet_granted:.1f}s ago); replanning")
            self._drop_plan()             # seeked away from it, or it came too late to use
        # bring a back-deck clip forward once it has had time to load and is playing in step
        for deck in self.decks:
            if deck is not front and deck.clip and deck.clip[0] == t.id and deck.clip[1] <= now < deck.clip[2] \
                    and (front is None or deck.clip[1] > front.clip[1]) \
                    and time.monotonic() - deck.published_at >= FLIP_AFTER:
                self._log(f"deck {deck.name} to the front at {now:.1f}s")
                self._set_front(deck)
                front = deck
                break
        self._plan_next(t, now)
        if not self.plan:
            return
        nxt = self.plan[0]
        if nxt.deck is front and len(self.decks) > 1:
            return
        # ask for the quiet window as soon as the encode is about done: building the long write takes a while
        if (nxt.fut.done() or now >= nxt.done_by - 0.5) \
                and now >= nxt.start - SHOW_LATENCY - QUIET_SETTLE - QUIET_BUILD and not p.quiet_requested \
                and p.quiet_granted < time.monotonic() - QUIET_HOLD:
            p.quiet_requested = True      # due shortly: have the write loop make room for the swap
            p.wake.set()
            self._log(f"quiet requested for {nxt.start:.1f}s at {now:.1f}s")
        elif nxt.fut.done() and now >= nxt.start - SHOW_LATENCY - 0.1 \
                and 0 <= time.monotonic() - p.quiet_granted < QUIET_HOLD - 1.0:
            self.plan.pop(0)
            try:
                path, frames = nxt.fut.result()
            except Exception as exc:
                print(f"clip encode failed: {exc!r}", file=sys.stderr, flush=True)
                return
            deck = nxt.deck
            self._publish(deck, Path(path))
            deck.clip, deck.blank, deck.published_at = (nxt.track, nxt.start, nxt.start + frames / FPS), False, time.monotonic()
            self._log(f"published {nxt.start:.1f}-{deck.clip[2]:.1f}s to deck {deck.name} at {now:.1f}s")
            if front is None or len(self.decks) == 1:
                self._set_front(deck)    # nothing on screen yet, or no second deck to hide the swap
            self._cleanup()

    def stop(self) -> None:
        self.pool.shutdown(wait=False, cancel_futures=True)
        for deck in self.decks:
            self._publish(deck, CLIPS / "glowtube-blank.svg")


def start_favicon_video(player: "Player") -> "FaviconVideo | None":
    """Favicon video needs a T3 token and the GlowTube project (GlowTube B makes swaps seamless);
    without them the theme video alone plays."""
    try:
        api = t3api.T3Api()
        a = api.find_project("GlowTube", str(HERE))
        b = api.find_project("GlowTube B", str(HERE / "deck-b"))
    except t3api.T3Error as exc:
        print(f"favicon video off: {exc}", flush=True)
        return None
    if a is None:
        print("favicon video off: no GlowTube project in T3", flush=True)
        return None
    decks = [Deck("a", a["id"])] + ([Deck("b", b["id"])] if b and b["id"] != a["id"] else [])
    print(f"favicon video on ({len(decks)} deck{'s' if len(decks) > 1 else ''})", flush=True)
    return FaviconVideo(player, api, decks)


# ---------------------------------------------------------------- theme file


def write_theme(css: str) -> None:
    doc = {"version": 1, "name": "GlowTube", "appearance": "dark", "canvas": "#07060b", "accent": "#ff2bd6",
           "css": css}
    THEMES_DIR.mkdir(parents=True, exist_ok=True)
    dest = THEMES_DIR / f"{THEME_ID}.json"
    tmp = THEMES_DIR / f"{THEME_ID}.json.glowtube-tmp"
    tmp.write_text(json.dumps(doc), encoding="utf-8")
    for _ in range(20):
        try:
            os.replace(tmp, dest)
            return
        except PermissionError:      # the server is reading it at this moment
            time.sleep(0.005)
    tmp.unlink(missing_ok=True)


def idle_theme(player: "Player | None" = None) -> str:
    if player is None:
        c = card_image.render("Nothing playing", "Copy a YouTube link, then press Play", False, [])
        card = theme_css.card_height_css(c.height) + theme_css.card_image_css(c)
    else:
        t = player.track
        card = player.card("Loading…" if t else "Copy a YouTube link, then press Play")
    return STATIC + theme_css.idle_css() + card


# ---------------------------------------------------------------- Windows input: clipboard and media keys

user32 = ctypes.WinDLL("user32", use_last_error=True) if os.name == "nt" else None
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True) if os.name == "nt" else None


def read_clipboard_text() -> str | None:
    user32.OpenClipboard.argtypes = [wintypes.HWND]
    user32.GetClipboardData.restype = ctypes.c_void_p
    kernel32.GlobalLock.argtypes = [ctypes.c_void_p]
    kernel32.GlobalLock.restype = ctypes.c_void_p
    kernel32.GlobalUnlock.argtypes = [ctypes.c_void_p]
    if not user32.OpenClipboard(None):
        return None
    try:
        handle = user32.GetClipboardData(13)  # CF_UNICODETEXT
        if not handle:
            return None
        ptr = kernel32.GlobalLock(handle)
        if not ptr:
            return None
        try:
            return ctypes.wstring_at(ptr)
        finally:
            kernel32.GlobalUnlock(handle)
    finally:
        user32.CloseClipboard()


def _in_right_panel(x: int) -> bool:
    """The card sits in T3's right-hand panel; clicks anywhere else (thread rows, the chat) never count."""
    hwnd = user32.GetForegroundWindow()
    r = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    scale = user32.GetDpiForWindow(hwnd) / 96 if hasattr(user32, "GetDpiForWindow") else 1.0
    return x >= r.right - 340 * scale


def _foreground_is_t3() -> bool:
    """True when the window in front belongs to a T3 Code process."""
    hwnd = user32.GetForegroundWindow()
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    handle = kernel32.OpenProcess(0x1000, False, pid.value)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        return False
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wintypes.DWORD(len(buf))
        if not kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size)):
            return False
        return "t3 code" in Path(buf.value).name.lower()
    finally:
        kernel32.CloseHandle(handle)


def _matches(px: np.ndarray, rgb, tol: int = 14) -> np.ndarray:
    return (np.abs(px - np.array(rgb)) <= tol).all(axis=-1)


def read_click(x: int, y: int) -> tuple[str, float] | None:
    """What the Now Playing card shows under a click at screen point (x, y), if anything.

    Grabs a thin strip of pixels around the cursor, looks for the button marker colors right under
    it, then for the progress bar's two colors on the cursor's row. The pixels are discarded."""
    from PIL import ImageGrab
    half, rows = 600, 8           # 17 rows: shorter than a queue row (24 px), well inside a disc (28 px)
    strip = np.asarray(ImageGrab.grab(bbox=(x - half, y - rows, x + half, y + rows + 1), all_screens=True)
                       .convert("RGB"), np.int16)
    cx, cy = half, rows
    # A control is a solid area of one marker color, so require most of a wide row band around the
    # cursor to match, not a few stray pixels (busy video frames contain near matches by chance).
    band = strip[:, cx - 14:cx + 15]
    hits = {name: int(_matches(band, rgb, tol=10).sum()) for name, rgb in card_image.MARKS.items()}
    name, count = max(hits.items(), key=lambda kv: kv[1])
    if count >= float(os.environ.get("GLOWTUBE_CLICK_FILL", "0.32")) * band.shape[0] * band.shape[1]:
        return name, 0.0
    for dy in sorted(range(-rows, rows + 1), key=abs):
        row = strip[cy + dy]
        on_bar = _matches(row, card_image.MARK_TRACK) | _matches(row, card_image.MARK_FILL)
        if not on_bar[cx]:
            continue
        left = right = cx
        while left > 0 and on_bar[left - 1]:
            left -= 1
        while right < len(row) - 1 and on_bar[right + 1]:
            right += 1
        if right - left > 60:
            return "scrub", (cx - left) / (right - left)
    return None


def card_action(player: Player, name: str, frac: float) -> None:
    if name == "scrub":
        player.seek_to(frac)
        player.wake.set()
    elif name == "toggle":
        command(player, "toggle", "")
    elif name.startswith("row"):
        player.play_next(int(name[3:]))
    elif name in ("linkplay", "linkqueue"):
        match = YOUTUBE.search((read_clipboard_text() or "")[:4000])
        if match:
            player.add(match.group(0), "now" if name == "linkplay" else "end")
        else:
            player.say("Copy a YouTube link first")


def watch_clicks(player: Player) -> None:
    """Makes the card's buttons and progress bar work: left clicks inside T3 are checked against them."""
    user32.GetAsyncKeyState.restype = ctypes.c_short
    kernel32.OpenProcess.restype = wintypes.HANDLE
    was_down = False
    while player.running:
        time.sleep(0.012)
        down = bool(user32.GetAsyncKeyState(0x01) & 0x8000)
        if down and not was_down:
            pt = wintypes.POINT()
            user32.GetCursorPos(ctypes.byref(pt))
            try:
                if _foreground_is_t3() and _in_right_panel(pt.x):
                    hit = read_click(pt.x, pt.y)
                    if hit:
                        name, frac = hit
                        print(f"card click: {name}{f' {frac:.2f}' if name == 'scrub' else ''}", flush=True)
                        card_action(player, name, frac)
            except Exception as exc:
                print(f"click check failed: {exc!r}", file=sys.stderr, flush=True)
        was_down = down


def watch_media_keys(player: Player) -> None:
    keys = {1: (0xB3, "toggle"), 2: (0xB0, "next"), 3: (0xB1, "prev")}
    for hotkey_id, (vk, _) in keys.items():
        if not user32.RegisterHotKey(None, hotkey_id, 0x4000, vk):  # MOD_NOREPEAT
            print(f"media key {vk:#x} is taken by another app", flush=True)
    msg = wintypes.MSG()
    while player.running and user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
        if msg.message == 0x0312 and msg.wParam in keys:  # WM_HOTKEY
            command(player, keys[msg.wParam][1], "")


# ---------------------------------------------------------------- control


def command(player: Player, cmd: str, arg: str) -> bool:
    with player.lock:
        if cmd == "toggle":
            player.playing = not player.playing
        elif cmd in ("play", "pause"):
            player.playing = cmd == "play"
        elif cmd == "next":
            player.jump(1)
        elif cmd == "prev":
            player.jump(-1)
        elif cmd == "seek":
            player.seek(float(arg or 10))
        elif cmd == "vol":
            player.volume = float(np.clip(float(arg), 0, 1.5))
        elif cmd == "chaos":
            player.chaos = float(np.clip(float(arg), 0, 11))
        elif cmd == "offset":
            player.offset += float(arg)
        elif cmd == "add":
            player.add(arg, "now")
        elif cmd == "queue":
            player.add(arg, "end")
        elif cmd == "playnext":
            player.play_next(int(arg or 1))
        elif cmd == "favicon":
            player.favicon_enabled = arg != "off"
        elif cmd == "holdwrites":
            player.hold_writes_until = time.monotonic() + float(arg or 2)
        elif cmd == "quit":
            player.running = False
        elif cmd != "status":
            return False
    if cmd != "status":
        player.wake.set()
    return True


def serve(player: Player) -> ThreadingHTTPServer:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            u = urllib.parse.urlparse(self.path)
            q = dict(urllib.parse.parse_qsl(u.query))
            if not command(player, u.path.strip("/"), q.get("v", "")):
                self.send_error(404, "unknown command")
                return
            with player.lock:
                t = player.track
                body = json.dumps({
                    "track": t.title if t else None, "playing": player.playing, "status": player.status,
                    "time": round(player.media_time(), 1), "queue": [x.title for x in player.queue],
                    "index": player.index, "volume": player.volume, "chaos": player.chaos,
                    "offset": round(player.offset, 3), "quality": QUALITY,
                }).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", CTL_PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


# ---------------------------------------------------------------- main


def play(urls: list[str]) -> None:
    if user32:
        # physical pixels everywhere, so cursor positions and screen grabs agree on scaled displays
        user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
    AHEAD.start()
    player = Player(urls)
    server = serve(player)
    favicon = start_favicon_video(player)
    if user32:
        threading.Thread(target=watch_media_keys, args=(player,), daemon=True).start()
        threading.Thread(target=watch_clicks, args=(player,), daemon=True).start()
    print(f"GlowTube running. Copy a YouTube link, then press Play or Queue on the card. Control port {CTL_PORT}.",
          flush=True)
    write_theme(idle_theme(player))
    last_css = None
    started = time.perf_counter()
    try:
        while player.running:
            t_build = time.perf_counter()
            try:
                css = player.batch()
            except Exception as exc:   # one bad batch must not stop the music
                print(f"batch failed: {exc!r}", file=sys.stderr, flush=True)
                css = None
            # the frames must start where the audio will be once this write is built and shown
            player.build_est = 0.7 * player.build_est + 0.3 * (time.perf_counter() - t_build)
            if css is None:
                t = player.track
                css = idle_theme(player)
            if css != last_css and time.monotonic() >= player.hold_writes_until:
                write_theme(css)
                last_css = css
                if player.quiet_requested and player.batch_quiet:
                    player.quiet_requested = player.batch_quiet = False
                    if os.environ.get("GLOWTUBE_DEBUG"):
                        print(f"[quiet] long write built in {time.perf_counter() - t_build:.2f}s", flush=True)
                    # T3 applies the write about 0.1 s after it lands; then nothing until the hold ends
                    player.hold_writes_until = time.monotonic() + QUIET_HOLD
                    player.quiet_granted = time.monotonic() + QUIET_SETTLE
            written = time.perf_counter()
            # T3 reloads 100 ms after the last change, so never write faster than MIN_GAP
            player.wake.wait(player.until_next())
            player.wake.clear()
            time.sleep(max(0.0, MIN_GAP - (time.perf_counter() - written)))
            started = time.perf_counter()
    except KeyboardInterrupt:
        pass
    finally:
        player.running = False
        player.stream.stop()
        if player.reader:
            player.reader.stop()
        if favicon:
            favicon.stop()
        AHEAD.stop()
        server.shutdown()
        write_theme(idle_theme())
        print("stopped", flush=True)


def ctl(args: list[str]) -> None:
    cmd = args[0] if args else "status"
    arg = args[1] if len(args) > 1 else ""
    url = f"http://127.0.0.1:{CTL_PORT}/{cmd}?" + urllib.parse.urlencode({"v": arg})
    print(urllib.request.urlopen(url, timeout=5).read().decode())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["play", "ctl", "idle"])
    parser.add_argument("args", nargs="*")
    ns = parser.parse_args()
    if ns.command == "play":
        play(ns.args)
    elif ns.command == "ctl":
        ctl(ns.args)
    else:
        write_theme(idle_theme())


if __name__ == "__main__":
    main()
