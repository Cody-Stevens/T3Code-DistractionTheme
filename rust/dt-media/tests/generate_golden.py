"""Regenerate the small analysis oracle without importing the running helper or touching T3.

Run from any directory: python rust/dt-media/tests/generate_golden.py
Requires ffmpeg and NumPy. Only the reference analyse AST is executed.
"""

import ast
import json
from pathlib import Path
import subprocess
from types import SimpleNamespace

import numpy as np

HERE = Path(__file__).resolve().parent
REFERENCE = HERE.parents[2] / "python" / "glowtube.py"
# Two different sweeps plus periodic decaying bass bursts, on different stereo channels.
SIGNAL = (
    "aevalsrc="
    "0.16*sin(2*PI*(40*t+875*t*t))"
    "+0.65*sin(2*PI*58*t)*exp(-35*mod(t\\,0.5))"
    "|0.12*sin(2*PI*(90*t+650*t*t))"
    "+0.55*sin(2*PI*71*t)*exp(-40*mod(t+0.09\\,0.75))"
    ":s=48000:d=4"
)


def main():
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-f", "lavfi", "-i", SIGNAL,
         "-f", "s16le", "-ac", "2", "-ar", "48000", "-"],
        capture_output=True, check=True,
    ).stdout
    pcm = np.frombuffer(raw, dtype="<i2").reshape(-1, 2)
    module = ast.parse(REFERENCE.read_text(encoding="utf-8"))
    function = next(node for node in module.body if isinstance(node, ast.FunctionDef) and node.name == "analyse")
    namespace = {"np": np, "SR": 48000, "FPS": 24, "theme_css": SimpleNamespace(BANDS=16)}
    exec(compile(ast.Module(body=[function], type_ignores=[]), str(REFERENCE), "exec"), namespace)
    features = namespace["analyse"](pcm)
    import hashlib
    fnv = 0xCBF29CE484222325
    for byte in raw:
        fnv = ((fnv ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    fixture = {
        "source": "python/glowtube.py:analyse",
        "numpy": np.__version__,
        "lavfi": SIGNAL,
        "pcm_frames": len(pcm),
        "pcm_sha256": hashlib.sha256(raw).hexdigest(),
        "pcm_fnv1a64": f"{fnv:016x}",
        "envelope_absolute_tolerance": 0.0002,
        **{key: np.round(value.astype(np.float64), 7).tolist() for key, value in features.items()},
    }
    (HERE / "analysis_golden.json").write_text(json.dumps(fixture, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"Wrote {len(pcm)} PCM frames, {len(features['bass'])} feature frames")


if __name__ == "__main__":
    main()
