"""Count theme writes (changes to the GlowTube theme file) over a period.

  python tools/write_rate.py <seconds>
"""
import os
import statistics
import sys
import time

PATH = os.path.expanduser("~/.t3/userdata/themes/glowtube.json")


def main(seconds: float) -> None:
    last, times, t0 = None, [], time.time()
    while time.time() - t0 < seconds:
        try:
            mtime = os.stat(PATH).st_mtime
        except OSError:
            mtime = None
        if mtime != last:
            last = mtime
            times.append(time.time())
        time.sleep(0.02)
    gaps = [b - a for a, b in zip(times, times[1:])]
    if gaps:
        print(f"{len(times)} writes in {seconds:.0f} s; gap median {statistics.median(gaps):.2f} s, max {max(gaps):.2f} s")
    else:
        print(f"{len(times)} writes in {seconds:.0f} s")


if __name__ == "__main__":
    main(float(sys.argv[1]) if len(sys.argv) > 1 else 30)
