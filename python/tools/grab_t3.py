"""Save screenshots of the T3 Code window, silently, for before/after comparisons.

  python tools/grab_t3.py <out-prefix> [count] [interval-seconds]

Finds the visible top-level window owned by a "T3 Code" process and grabs its screen rectangle.
The window must be on top (anything covering it is captured instead).
"""

import ctypes
import sys
import time
from ctypes import wintypes
from pathlib import Path

from PIL import ImageGrab

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
kernel32.OpenProcess.restype = wintypes.HANDLE


def process_name(hwnd) -> str:
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    handle = kernel32.OpenProcess(0x1000, False, pid.value)
    if not handle:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wintypes.DWORD(len(buf))
        kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size))
        return Path(buf.value).name.lower()
    finally:
        kernel32.CloseHandle(handle)


def find_t3() -> tuple[int, int, int, int]:
    found = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def visit(hwnd, _):
        if user32.IsWindowVisible(hwnd) and "t3 code" in process_name(hwnd):
            r = wintypes.RECT()
            user32.GetWindowRect(hwnd, ctypes.byref(r))
            if r.right - r.left > 400 and r.bottom - r.top > 300:
                found.append((r.left, r.top, r.right, r.bottom))
        return True

    user32.EnumWindows(visit, 0)
    if not found:
        raise SystemExit("no visible T3 Code window")
    return max(found, key=lambda r: (r[2] - r[0]) * (r[3] - r[1]))


def main() -> None:
    prefix = Path(sys.argv[1])
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    interval = float(sys.argv[3]) if len(sys.argv) > 3 else 0.5
    prefix.parent.mkdir(parents=True, exist_ok=True)
    box = find_t3()
    for i in range(count):
        path = prefix.with_name(f"{prefix.name}-{i}.png")
        ImageGrab.grab(bbox=box, all_screens=True).save(path)
        print(path)
        if i + 1 < count:
            time.sleep(interval)


if __name__ == "__main__":
    main()
