"""Run the write loop in real time against a simulated clock (no audio, no theme file) and report
how often the encode-ahead clips arrive in time and what quality the video gets."""
import re
import sys
import threading
import time
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import glowtube as g  # noqa: E402


class SimPlayer(g.Player):
    def __init__(self, track, at):
        self.queue, self.index, self.lock, self.playing = [track], 0, threading.RLock(), True
        self.reader, self.chaos, self.offset, self.build_est = None, 11, 0.16, 0.05
        self.status, self.status_until = "", 0.0
        self._card_key, self._card = None, None
        self.wake = threading.Event()
        self.t0, self.at, self.next_k = time.perf_counter(), at, None

    def media_time(self):
        return self.at + time.perf_counter() - self.t0


def main():
    t = g.Track("https://www.youtube.com/watch?v=fDltPLFdkYI")
    g.load(t)
    g.AHEAD.start()
    for at, seconds in ((70, 8), (150, 8)):
        p = SimPlayer(t, at)
        kinds, builds, sizes = Counter(), [], []
        end = time.perf_counter() + seconds
        while time.perf_counter() < end:
            started = time.perf_counter()
            css = p.batch()
            builds.append((time.perf_counter() - started) * 1000)
            p.build_est = 0.7 * p.build_est + 0.3 * builds[-1] / 1000
            if css:
                kinds["ahead" if "ahead" in g.QUALITY else "now"] += 1
                sizes.append(len(css))
                q = re.search(r"(\d+x\d+) 24fps q(\d+)", g.QUALITY)
                kinds[f"{q.group(1)} q{q.group(2)}"] += 1
            time.sleep(max(g.MIN_GAP - (time.perf_counter() - started), p.until_next()))
        p.reader.stop()
        g.AHEAD.clear()
        builds.sort()
        print(f"t={at}s: {dict(kinds)}  build median {builds[len(builds) // 2]:.0f} ms, max {builds[-1]:.0f} ms, "
              f"largest write {max(sizes)}  ahead stats {g.AHEAD.stats}")
        g.AHEAD.stats = dict.fromkeys(g.AHEAD.stats, 0)
    g.AHEAD.stop()


if __name__ == "__main__":
    main()
