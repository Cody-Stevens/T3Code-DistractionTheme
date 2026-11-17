"""Build a few real writes offline (no theme file is touched) and report their size, video level and build time."""
import re
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import glowtube as g  # noqa: E402

t = g.Track("https://www.youtube.com/watch?v=fDltPLFdkYI")
g.load(t)


class Offline(g.Player):
    def __init__(self, track, at):
        self.queue, self.index, self.lock, self.playing = [track], 0, threading.RLock(), True
        self.pos, self.reader, self.chaos, self.offset, self.build_est = int(at * g.SR), None, 11, 0.16, 0.06
        self.status, self.status_until = "", 0.0
        self._card_key, self._card = None, None
        self.wake = threading.Event()

    def media_time(self):
        return self.pos / g.SR


for at in (32, 90, 154):
    p = Offline(t, at)
    p.batch()
    time.sleep(1.5)
    for i in range(6):
        t0 = time.perf_counter()
        css = p.batch()
        ms = (time.perf_counter() - t0) * 1000
        parts = [name for name, pat in (("glow", "--glow:url"), ("card", "--cardimg:url")) if pat in css]
        vid = len(re.search(r'--vid:url\("([^"]+)"', css).group(1))
        print(f"t={at + i * g.BATCH:6.2f}s {ms:4.0f} ms  css {len(css):5}  video {vid:5} chars at {g.QUALITY:<18} {'+'.join(parts)}")
        p.pos += int(g.BATCH * g.SR)
        time.sleep(0.05)
    p.reader.stop()
