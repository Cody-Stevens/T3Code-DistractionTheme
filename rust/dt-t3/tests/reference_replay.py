"""Replay only Python's scheduler definitions with fake clocks, futures and API effects.

Run from rust/: python dt-t3/tests/reference_replay.py
No helper imports, processes, media, network or user files are used.
"""

import ast
import math
import sys
from dataclasses import dataclass
from pathlib import Path
from types import SimpleNamespace


clock = SimpleNamespace(wall=100.0, media=0.0)
constants = {
    "CLIP_SECONDS", "FIRST_CLIP_SECONDS", "FIRST_LEAD", "HANDOVER", "FLIP_AFTER",
    "SHOW_LATENCY", "QUIET_BUILD", "PUBLISH_LEAD", "ENCODE_COST", "ENCODE_START",
    "QUIET_SETTLE", "QUIET_HOLD", "GATE", "GATE_MAX",
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
    dataclass=dataclass, Path=Path, Future=object, Track=object, math=math,
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
    jumps=0, gate_release=None, stream=SimpleNamespace(latency=0.1),
    media_time=lambda: clock.media, wake=SimpleNamespace(set=lambda: None),
)
scheduler = namespace["FaviconVideo"].__new__(namespace["FaviconVideo"])
scheduler.player = player
scheduler.decks = [namespace["Deck"]("a", "project-a"), namespace["Deck"]("b", "project-b")]
scheduler.front, scheduler.plan, scheduler.seq, scheduler.run_id = None, [], 0, "fake-run"
scheduler.gate_started, scheduler.jumps_seen = 0.0, 0
scheduler.pool = Pool()
scheduler._new_pool = lambda: setattr(scheduler, "pool", Pool())
scheduler._log = lambda *_: None
scheduler._cleanup = lambda: None
failed_handover = "--failed-handover" in sys.argv
attempts, failed_deck, old_deck_state = 0, None, None

def publish(deck, path):
    global attempts, failed_deck, old_deck_state
    attempts += 1
    if failed_handover and attempts == 4:
        failed_deck, old_deck_state = deck, (deck.clip, deck.file)
        return False
    publishes.append((clock.media, deck.name, str(path), clock.wall))
    return True
scheduler._publish = publish
grant_due = None
samples = 0
previous_front = None
tenth = 0
gaps = []
while clock.media < 300.0:
    clock.wall = 100.0 + tenth / 10
    if grant_due is not None and clock.wall >= grant_due - 1e-8:
        player.quiet_granted = grant_due + namespace["QUIET_SETTLE"]
        player.quiet_requested, grant_due = False, None
    if scheduler.front and clock.media > scheduler.front.clip[2]:
        gaps.append(clock.media - scheduler.front.clip[2])
    scheduler._tick()
    if failed_deck is not None:
        assert (failed_deck.clip, failed_deck.file) == old_deck_state
        failed_deck = None
    if player.quiet_requested and grant_due is None:
        quiet_requests.append(clock.media)
        grant_due = clock.wall + 0.6
    if scheduler.front is not previous_front:
        flips.append((clock.media, scheduler.front.name if scheduler.front else None))
        previous_front = scheduler.front
    audible_at = player.gate_release + player.stream.latency if player.gate_release is not None else 0
    if clock.wall + 1e-8 >= audible_at:
        assert scheduler.front and scheduler.front.clip[1] <= clock.media < scheduler.front.clip[2]
        samples += 1
    if player.gate_release is not None and clock.wall + 1e-8 >= audible_at:
        player.gate_release = None
    delta = max(0, clock.wall + 0.1 - max(clock.wall, audible_at))
    clock.media = round(clock.media + delta, 9)
    tenth += 1

expected_lengths = [10.0, 10.839160839, 25.0, 25.0]
expected_starts = [0.0, 7.0, 14.839160839, 36.839160839]
for job, length, start in zip(jobs, expected_lengths, expected_starts):
    assert abs(job["duration"] - length) < 1e-8
    assert abs(job["start"] - start) < 1e-8
if failed_handover:
    assert len(gaps) == 1 and abs(gaps[0] - 0.06083916083916) < 1e-8, gaps
    print("Python failed-handover polling gap:", gaps[0])
else:
    assert not gaps
    assert [(at, deck) for at, deck, *_ in publishes[:4]] == [(0.0, "a"), (6.4, "b"), (14.3, "a"), (36.3, "b")]
    assert flips[:4] == [(0.0, "a"), (7.6, "b"), (15.5, "a"), (37.5, "b")]
print("Python planned durations:", [job["duration"] for job in jobs[:4]])
print("Python planned starts:", [job["start"] for job in jobs[:4]])
print("Python first publishes:", [(at, deck) for at, deck, *_ in publishes[:4]])
print("Python first flips:", flips[:4])
print("Python first quiet requests:", quiet_requests[:4])
print(f"Python covered polling samples: {samples}/{samples} through 300 seconds; interval gaps: {len(gaps)}")
