"""Replay only Python's scheduler definitions with fake clocks, futures and API effects.

Run from rust/: python dt-t3/tests/reference_replay.py
No helper imports, processes, media, network or user files are used.
"""

import ast
from dataclasses import dataclass
from pathlib import Path
from types import SimpleNamespace


clock = SimpleNamespace(wall=100.0, media=0.0)
constants = {
    "CLIP_SECONDS", "FIRST_CLIP_SECONDS", "FIRST_LEAD", "HANDOVER", "FLIP_AFTER",
    "SHOW_LATENCY", "QUIET_BUILD", "PUBLISH_LEAD", "ENCODE_COST", "ENCODE_START",
    "QUIET_SETTLE", "QUIET_HOLD",
}
source = Path(__file__).resolve().parents[3] / "python" / "glowtube.py"
tree = ast.parse(source.read_text(encoding="utf-8"))
nodes = [node for node in tree.body if (
    isinstance(node, ast.Assign)
    and any(isinstance(target, ast.Name) and target.id in constants for target in node.targets)
) or (isinstance(node, (ast.ClassDef, ast.FunctionDef)) and node.name in {
    "Deck", "Planned", "FaviconVideo", "encode_seconds",
})]
namespace = dict(
    dataclass=dataclass, Path=Path, Future=object, Track=object,
    t3api=SimpleNamespace(T3Api=object), os=SimpleNamespace(environ={}),
    time=SimpleNamespace(monotonic=lambda: clock.wall),
    encoder=SimpleNamespace(encode_clip_svg=object()),
    CLIPS=Path("synthetic-clips-no-io"), FPS=24, FRAME_W=640, FRAME_H=360,
)
exec(compile(ast.Module(body=nodes, type_ignores=[]), str(source), "exec"), namespace)
jobs, publishes, flips, quiet_requests = [], [], [], []


class Future:
    def __init__(self, due, output, frames):
        self.due, self.output, self.frames = due, output, frames

    def done(self):
        return clock.wall >= self.due

    def result(self):
        return self.output, self.frames

    def cancel(self):
        return False


class Pool:
    free = clock.wall

    def submit(self, _function, _video, start, seconds, output, _fps, _size, _quality, speed):
        self.free = max(clock.wall, self.free) + 0.3 + seconds * (0.15 if speed == 10 else 0.22)
        jobs.append(dict(start=start, duration=seconds, output=output, speed=speed))
        return Future(self.free, output, round(seconds * 24))


player = SimpleNamespace(
    track=SimpleNamespace(id="synthetic", path="synthetic.mp4", frames=400 * 24, pcm=True),
    playing=True, favicon_enabled=True, quiet_requested=False, quiet_granted=-100.0,
    media_time=lambda: clock.media, wake=SimpleNamespace(set=lambda: None),
)
scheduler = namespace["FaviconVideo"].__new__(namespace["FaviconVideo"])
scheduler.player = player
scheduler.decks = [namespace["Deck"]("a", "project-a"), namespace["Deck"]("b", "project-b")]
scheduler.front, scheduler.plan, scheduler.seq, scheduler.run_id = None, [], 0, "fake-run"
scheduler.pool = Pool()
scheduler._log = lambda *_: None
scheduler._cleanup = lambda: None
scheduler._publish = lambda deck, path: publishes.append((clock.media, deck.name, str(path)))
grant_due = None
samples = 0
previous_front = None
for tenth in range(3001):
    clock.media, clock.wall = tenth / 10, 100.0 + tenth / 10
    if grant_due is not None and clock.wall >= grant_due - 1e-8:
        player.quiet_granted = grant_due + namespace["QUIET_SETTLE"]
        player.quiet_requested, grant_due = False, None
    scheduler._tick()
    if player.quiet_requested and grant_due is None:
        quiet_requests.append(clock.media)
        grant_due = clock.wall + 0.6
    if scheduler.front is not previous_front:
        flips.append((clock.media, scheduler.front.name if scheduler.front else None))
        previous_front = scheduler.front
    if clock.media >= 3.0:
        assert scheduler.front and scheduler.front.clip[1] <= clock.media < scheduler.front.clip[2]
        samples += 1

print("Python planned durations:", [job["duration"] for job in jobs[:4]])
print("Python planned starts:", [job["start"] for job in jobs[:4]])
print("Python first publishes:", [(at, deck) for at, deck, _ in publishes[:4]])
print("Python first flips:", flips[:4])
print("Python first quiet requests:", quiet_requests[:4])
print(f"Python steady coverage: {samples}/{samples} samples, 3.0 through 300.0 seconds")
