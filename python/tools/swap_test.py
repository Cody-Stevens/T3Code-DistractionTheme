"""Time favicon swaps in the desktop app: blank -> clip and clip -> clip, with whatever the helper is
doing to the theme at the time (run it paused and playing). The helper's own favicon video must be off."""
import os, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..")); sys.path.insert(0, os.path.dirname(__file__))
import numpy as np
from PIL import ImageGrab
import encoder, t3api
from grab_t3 import find_t3

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
api = t3api.T3Api(); pid = api.find_project("GlowTube", ROOT)["id"]
l, t, r, b = find_t3()
cx, cy = l + int((r - l) * 0.57), t + int((b - t) * 0.42)

def outline():
    a = np.asarray(ImageGrab.grab(bbox=(l, t, r, b), all_screens=True).convert("RGB")).astype(int)
    return ((a[..., 0] > 220) & (a[..., 1] < 40) & (a[..., 2] > 220)).sum() > 1000

def centre():
    return np.asarray(ImageGrab.grab(bbox=(cx - 40, cy - 40, cx + 40, cy + 40), all_screens=True).convert("RGB"), float)

def solid(color, name):
    path = os.path.join(ROOT, "clips", name)
    pad = "<!--" + "." * 4_400_000 + "-->"
    open(path, "w").write(f'<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360">{pad}<rect width="640" height="360" fill="{color}"/></svg>')
    return path

def wait(cond, limit=25):
    t0 = time.perf_counter()
    while time.perf_counter() - t0 < limit:
        if cond():
            return time.perf_counter() - t0
        time.sleep(0.1)
    return None

stamp = int(time.time())
blank = os.path.join(ROOT, "clips", "glowtube-blank.svg")
api.update_project(pid, faviconPath=blank); time.sleep(2)
a = solid("#00ff00", f"glowtube-clip-swapA-{stamp}.svg"); bpath = solid("#0000ff", f"glowtube-clip-swapB-{stamp}.svg")
hold = float(os.environ.get("HOLD", "0"))
def quiet():
    if hold:
        import urllib.request
        urllib.request.urlopen(f"http://127.0.0.1:47873/holdwrites?v={hold}", timeout=3).read()
        time.sleep(1.5)
quiet()
api.update_project(pid, faviconPath=a)
print("blank -> big green:", wait(lambda: outline() and centre()[..., 1].mean() > 200))
time.sleep(1.5)
quiet()
api.update_project(pid, faviconPath=bpath)
print("big green -> big blue:", wait(lambda: centre()[..., 2].mean() > 200 and centre()[..., 1].mean() < 60))
time.sleep(1)
api.update_project(pid, faviconPath=blank)
for p in (a, bpath):
    os.remove(p)
