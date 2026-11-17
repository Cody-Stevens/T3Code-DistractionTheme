use super::{error, last_error, owned_process_image};
use crate::input::{InputEvent, ScreenPoint, point_in_right_panel};
use dt_core::{Error, Frame, Result};
use std::sync::mpsc::Sender;
use windows::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{DataExchange::*, Memory::*},
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub(crate) fn enable_dpi_awareness() -> Result<()> {
    // SAFETY: documented pseudo-context constant; no pointers or ownership transfer.
    unsafe {
        if let Err(failure) =
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
        {
            if AreDpiAwarenessContextsEqual(
                GetThreadDpiAwarenessContext(),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
            .as_bool()
            {
                return Ok(());
            }
            return Err(error(failure));
        }
        Ok(())
    }
}

pub(crate) fn foreground_is_t3() -> bool {
    // SAFETY: read-only window queries with a valid stack output; disappearing windows yield 0.
    let pid = unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        pid
    };
    owned_process_image(pid).is_some_and(|path| {
        path.rsplit(['\\', '/'])
            .next()
            .unwrap_or("")
            .to_lowercase()
            .contains("t3 code")
    })
}

pub(crate) fn in_right_panel(point: ScreenPoint) -> Result<bool> {
    // SAFETY: borrowed foreground HWND and initialized RECT; failure never becomes a hit.
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return Ok(false);
        }
        let mut rect = RECT::default();
        GetWindowRect(window, &mut rect).map_err(error)?;
        Ok(point_in_right_panel(
            point,
            rect.right,
            GetDpiForWindow(window),
        ))
    }
}

pub(crate) fn left_button_down() -> bool {
    // SAFETY: read-only key query; the sign bit is current state, not the shared low bit.
    unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) < 0 }
}

pub(crate) fn cursor_position() -> Result<ScreenPoint> {
    let mut point = POINT::default();
    // SAFETY: output points to initialized stack storage.
    unsafe {
        GetCursorPos(&mut point).map_err(error)?;
    }
    Ok(ScreenPoint {
        x: point.x,
        y: point.y,
    })
}

struct Capture {
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Drop for Capture {
    fn drop(&mut self) {
        // SAFETY: only owned resources are released; restore selection before deleting bitmap.
        unsafe {
            if !self.previous.0.is_null() {
                SelectObject(self.memory, self.previous);
            }
            if !self.bitmap.0.is_null() {
                let _ = DeleteObject(self.bitmap.into());
            }
            if !self.memory.0.is_null() {
                let _ = DeleteDC(self.memory);
            }
            if !self.screen.0.is_null() {
                ReleaseDC(None, self.screen);
            }
        }
    }
}

pub(crate) fn capture_strip(point: ScreenPoint) -> Result<Frame> {
    let left = point
        .x
        .checked_sub(600)
        .ok_or(Error::InvalidInput("screen coordinate overflow"))?;
    let top = point
        .y
        .checked_sub(8)
        .ok_or(Error::InvalidInput("screen coordinate overflow"))?;
    let right = left
        .checked_add(1200)
        .ok_or(Error::InvalidInput("screen coordinate overflow"))?;
    let bottom = top
        .checked_add(17)
        .ok_or(Error::InvalidInput("screen coordinate overflow"))?;
    // SAFETY: handles are guarded from acquisition onward. The top-down 32-bit DIB allocates
    // 1200*17*4 bytes. GdiFlush completes drawing before the pixels are read.
    unsafe {
        let mut capture = Capture {
            screen: GetDC(None),
            memory: HDC::default(),
            bitmap: HBITMAP::default(),
            previous: HGDIOBJ::default(),
        };
        if capture.screen.0.is_null() {
            return Err(last_error());
        }
        capture.memory = CreateCompatibleDC(Some(capture.screen));
        if capture.memory.0.is_null() {
            return Err(last_error());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 1200,
                biHeight: -17,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        capture.bitmap = CreateDIBSection(
            Some(capture.screen),
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        )
        .map_err(error)?;
        if bits.is_null() {
            return Err(Error::Protocol("GDI strip has no pixels"));
        }
        capture.previous = SelectObject(capture.memory, capture.bitmap.into());
        if capture.previous.0.is_null() || capture.previous.0 as isize == -1 {
            capture.previous = HGDIOBJ::default();
            return Err(last_error());
        }
        std::ptr::write_bytes(bits.cast::<u8>(), 0, 1200 * 17 * 4);
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vr = vx.saturating_add(GetSystemMetrics(SM_CXVIRTUALSCREEN));
        let vb = vy.saturating_add(GetSystemMetrics(SM_CYVIRTUALSCREEN));
        let (sx, sy, ex, ey) = (left.max(vx), top.max(vy), right.min(vr), bottom.min(vb));
        if ex > sx && ey > sy {
            BitBlt(
                capture.memory,
                sx - left,
                sy - top,
                ex - sx,
                ey - sy,
                Some(capture.screen),
                sx,
                sy,
                SRCCOPY | CAPTUREBLT,
            )
            .map_err(error)?;
        }
        if !GdiFlush().as_bool() {
            return Err(last_error());
        }
        let bgra = std::slice::from_raw_parts(bits.cast::<u8>(), 1200 * 17 * 4);
        let rgb = bgra
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0]])
            .collect();
        Frame::new(1200, 17, rgb)
    }
}

struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        // SAFETY: guard exists only after a successful OpenClipboard on this thread.
        unsafe {
            let _ = CloseClipboard();
        }
    }
}
struct ClipboardLock(HGLOBAL);
impl Drop for ClipboardLock {
    fn drop(&mut self) {
        // SAFETY: clipboard is still open; this guard owns one successful GlobalLock.
        unsafe {
            let _ = GlobalUnlock(self.0);
        }
    }
}

pub(crate) fn read_clipboard_text() -> Result<Option<String>> {
    // SAFETY: clipboard stays open until after the lock drops. GlobalSize bounds the UTF-16
    // slice; malformed or unterminated data cannot cause a read past the allocation.
    unsafe {
        if OpenClipboard(None).is_err() {
            return Ok(None);
        }
        let _clipboard = Clipboard;
        let Ok(handle) = GetClipboardData(13) else {
            return Ok(None);
        };
        let global = HGLOBAL(handle.0);
        let bytes = GlobalSize(global);
        if bytes < 2 || bytes > isize::MAX as usize {
            return Ok(None);
        }
        let pointer = GlobalLock(global);
        if pointer.is_null() {
            return Ok(None);
        }
        let _lock = ClipboardLock(global);
        let text = std::slice::from_raw_parts(pointer.cast::<u16>(), bytes / 2);
        Ok(clipboard_string(text))
    }
}

const KEYS: [(u32, InputEvent); 3] = [
    (0xb3, InputEvent::Toggle),
    (0xb0, InputEvent::Next),
    (0xb1, InputEvent::Previous),
];
pub(crate) struct MediaKeys {
    registered: [bool; 3],
}
impl MediaKeys {
    pub(crate) fn register() -> Self {
        Self::register_with(|id, key| {
            // SAFETY: thread-scoped registration, private IDs 1..3, no window or pointer.
            unsafe { RegisterHotKey(None, id, MOD_NOREPEAT, key) }.is_ok()
        })
    }
    fn register_with(mut register: impl FnMut(i32, u32) -> bool) -> Self {
        let mut registered = [false; 3];
        for (i, (key, _)) in KEYS.iter().enumerate() {
            registered[i] = register(i as i32 + 1, *key);
            if !registered[i] {
                eprintln!("media key {key:#x} is taken by another app");
            }
        }
        Self { registered }
    }
    pub(crate) fn dispatch(&self, events: &Sender<InputEvent>) -> bool {
        let mut message = MSG::default();
        // Bound each tick so a message flood cannot delay stop.
        for _ in 0..64 {
            // SAFETY: initialized MSG output; this thread's queue is polled without blocking.
            if !unsafe { PeekMessageW(&mut message, None, WM_HOTKEY, WM_HOTKEY, PM_REMOVE) }
                .as_bool()
            {
                break;
            }
            if let Some(event) = self.event(message.wParam.0)
                && events.send(event).is_err()
            {
                return false;
            }
        }
        true
    }
    fn event(&self, id: usize) -> Option<InputEvent> {
        let index = id.checked_sub(1).filter(|&index| index < 3)?;
        self.registered[index].then_some(KEYS[index].1)
    }
}
impl Drop for MediaKeys {
    fn drop(&mut self) {
        for (i, registered) in self.registered.iter().enumerate() {
            if *registered {
                // SAFETY: only successful registrations on this same worker thread are unregistered.
                unsafe {
                    let _ = UnregisterHotKey(None, i as i32 + 1);
                }
            }
        }
    }
}

fn clipboard_string(text: &[u16]) -> Option<String> {
    let end = text.iter().position(|&c| c == 0)?;
    Some(String::from_utf16_lossy(&text[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taken_hotkeys_do_not_fail_and_unregistered_ids_do_not_dispatch() {
        let mut attempts = Vec::new();
        let mut keys = MediaKeys::register_with(|id, key| {
            attempts.push((id, key));
            id == 2
        });
        assert_eq!(attempts, [(1, 0xb3), (2, 0xb0), (3, 0xb1)]);
        assert_eq!(keys.event(0), None);
        assert_eq!(keys.event(1), None);
        assert_eq!(keys.event(2), Some(InputEvent::Next));
        assert_eq!(keys.event(3), None);
        assert_eq!(keys.event(usize::MAX), None);
        // Synthetic registrations own no Win32 hotkeys, so Drop must not unregister anything.
        keys.registered = [false; 3];
    }

    #[test]
    fn clipboard_utf16_is_bounded_terminated_and_handles_surrogates() {
        assert_eq!(clipboard_string(&[0]), Some(String::new()));
        assert_eq!(
            clipboard_string(&[65, 0xd83d, 0xde80, 0, 66]),
            Some("A🚀".into())
        );
        assert_eq!(clipboard_string(&[0xd800, 0]), Some("�".into()));
        assert_eq!(clipboard_string(&[65]), None);
        assert_eq!(clipboard_string(&[]), None);
    }
}
