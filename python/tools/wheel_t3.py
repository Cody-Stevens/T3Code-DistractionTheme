"""Scroll the T3 Code chat by posting mouse-wheel messages (no clicks, no cursor move, no focus change).

  python tools/wheel_t3.py <notches> [x-fraction y-fraction]

Positive notches scroll up (back in the thread), negative scroll down. The point defaults to the
chat column, 57% across and 75% down the window.
"""
import ctypes
import sys
import time
from ctypes import wintypes

sys.path.insert(0, __file__.rsplit("\\", 1)[0])
import grab_t3  # noqa: E402

user32 = grab_t3.user32
WM_MOUSEWHEEL = 0x020A


def t3_hwnd():
    found = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def visit(hwnd, _):
        if user32.IsWindowVisible(hwnd) and "t3 code" in grab_t3.process_name(hwnd):
            r = wintypes.RECT()
            user32.GetWindowRect(hwnd, ctypes.byref(r))
            if r.right - r.left > 400:
                found.append((hwnd, r))
        return True

    user32.EnumWindows(visit, 0)
    return max(found, key=lambda f: (f[1].right - f[1].left) * (f[1].bottom - f[1].top))


def main():
    notches = int(sys.argv[1])
    fx, fy = (float(v) for v in sys.argv[2:4]) if len(sys.argv) > 3 else (0.57, 0.75)
    hwnd, r = t3_hwnd()
    x, y = int(r.left + (r.right - r.left) * fx), int(r.top + (r.bottom - r.top) * fy)
    step = 120 if notches > 0 else -120
    for _ in range(abs(notches)):
        wparam = (step & 0xFFFF) << 16
        lparam = ((y & 0xFFFF) << 16) | (x & 0xFFFF)
        user32.PostMessageW(hwnd, WM_MOUSEWHEEL, wparam, lparam)
        time.sleep(0.05)


if __name__ == "__main__":
    main()
