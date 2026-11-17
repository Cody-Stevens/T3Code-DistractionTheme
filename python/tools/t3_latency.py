"""How long T3's desktop client waits for its own server calls, from T3's trace log.

  python tools/t3_latency.py <seconds>

Reports the client-side duration of assets.createUrl and preview.reportProfiles calls started in
the last <seconds>. The server answers them in about a millisecond, so a long client-side duration
means the client's main thread was too busy to handle the reply (measured: ~2 ms with the theme
idle, ~2 s with the theme writing 4 times a second)."""
import json
import os
import statistics
import sys
import time
from pathlib import Path

LOGS = Path(os.path.expanduser("~")) / ".t3" / "userdata" / "logs"


def main(seconds: float) -> None:
    since = time.time() - seconds
    durations = []
    for name in ("server.trace.ndjson.1", "server.trace.ndjson"):
        path = LOGS / name
        if not path.exists():
            continue
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if '"kind":"client"' not in line or ("assets.createUrl" not in line and "preview.reportProfiles" not in line):
                    continue
                try:
                    d = json.loads(line)
                except ValueError:
                    continue
                if int(d["startTimeUnixNano"]) / 1e9 >= since:
                    durations.append(d["durationMs"])
    if durations:
        print(f"{len(durations)} calls: median {statistics.median(durations):.0f} ms, max {max(durations):.0f} ms")
    else:
        print("no calls in that window")


if __name__ == "__main__":
    main(float(sys.argv[1]) if len(sys.argv) > 1 else 30)
