//! Physical-pixel input from Python's click, clipboard and media-key watchers.

use dt_core::{
    CardTarget, Error, Frame, Result,
    constants::{CARD_MARKS, MARK_FILL, MARK_TRACK},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread::JoinHandle,
};
#[cfg(any(windows, test))]
use std::{thread, time::Duration};

/// Physical screen coordinates, including negative positions on multiple monitors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenPoint {
    /// Physical horizontal pixel coordinate.
    pub x: i32,
    /// Physical vertical pixel coordinate.
    pub y: i32,
}

/// Port `read_click`'s `(name, fraction)` result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardHit {
    /// Marker target or scrub bar.
    pub target: CardTarget,
    /// 0..=1 for scrub, zero for buttons/rows.
    pub fraction: f32,
}

/// Events delivered to the player command thread without holding its lock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    /// Global media play/pause key.
    Toggle,
    /// Global media next key.
    Next,
    /// Global media previous key.
    Previous,
    /// Verified click inside T3's right panel.
    Card(CardHit),
}

/// Thresholds from `GLOWTUBE_CLICK_FILL` and `read_click`.
#[derive(Clone, Copy, Debug)]
pub struct ClickConfig {
    /// Default .32 of a 29x17 band, within 10/channel of one exact marker RGB.
    pub minimum_fill: f32,
    /// Progress-bar tolerance per RGB channel, default 14; require >60 contiguous pixels.
    pub bar_tolerance: u8,
}

impl Default for ClickConfig {
    fn default() -> Self {
        Self {
            minimum_fill: 0.32,
            bar_tolerance: 14,
        }
    }
}

impl ClickConfig {
    /// Load the optional fill override. Reject non-finite or out-of-range thresholds.
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();
        match std::env::var("GLOWTUBE_CLICK_FILL") {
            Ok(value) => {
                config.minimum_fill = value
                    .parse()
                    .map_err(|_| Error::InvalidInput("GLOWTUBE_CLICK_FILL"))?
            }
            Err(std::env::VarError::NotPresent) => {}
            Err(_) => return Err(Error::InvalidInput("GLOWTUBE_CLICK_FILL")),
        }
        config.validate()?;
        Ok(config)
    }

    fn validate(self) -> Result<()> {
        if !self.minimum_fill.is_finite() || !(0.0..=1.0).contains(&self.minimum_fill) {
            return Err(Error::InvalidInput(
                "click fill must be finite and in 0..=1",
            ));
        }
        Ok(())
    }
}

const TARGETS: [CardTarget; 7] = [
    CardTarget::Toggle,
    CardTarget::LinkPlay,
    CardTarget::LinkQueue,
    CardTarget::QueueRow(0),
    CardTarget::QueueRow(1),
    CardTarget::QueueRow(2),
    CardTarget::QueueRow(3),
];
#[cfg(any(windows, test))]
const POLL: Duration = Duration::from_millis(12);

fn matches(pixel: &[u8], marker: [u8; 3], tolerance: u8) -> bool {
    pixel
        .iter()
        .zip(marker)
        .all(|(&a, b)| a.abs_diff(b) <= tolerance)
}

/// Pure `read_click` hit test. `cursor` is relative to the RGB24 image's upper-left corner.
/// Requires a complete 29x17 band; no clipping, shrinking or DPI scaling of that band.
/// Button ties follow Python marker insertion order. Scrub rows scan 0,-1,+1,...,-8,+8.
pub fn hit_test(
    pixels: &Frame,
    cursor: ScreenPoint,
    config: ClickConfig,
) -> Result<Option<CardHit>> {
    config.validate()?;
    let (x, y) = (i64::from(cursor.x), i64::from(cursor.y));
    if x < 14 || y < 8 || x + 14 >= i64::from(pixels.width()) || y + 8 >= i64::from(pixels.height())
    {
        return Ok(None);
    }
    let width = pixels.width() as usize;
    let (x, y) = (x as usize, y as usize);
    let pixel = |x: usize, y: usize| &pixels.pixels()[(y * width + x) * 3..(y * width + x) * 3 + 3];
    let mut best = (0, 0);
    for (index, marker) in CARD_MARKS.into_iter().enumerate() {
        let count = (y - 8..=y + 8)
            .flat_map(|py| (x - 14..=x + 14).map(move |px| (px, py)))
            .filter(|&(px, py)| matches(pixel(px, py), marker, 10))
            .count();
        if count > best.1 {
            best = (index, count);
        }
    }
    if best.1 as f32 >= config.minimum_fill * (29 * 17) as f32 {
        return Ok(Some(CardHit {
            target: TARGETS[best.0],
            fraction: 0.0,
        }));
    }
    for dy in [0, -1, 1, -2, 2, -3, 3, -4, 4, -5, 5, -6, 6, -7, 7, -8, 8] {
        let py = (y as i64 + dy) as usize;
        let on_bar = |px| {
            matches(pixel(px, py), MARK_TRACK, config.bar_tolerance)
                || matches(pixel(px, py), MARK_FILL, config.bar_tolerance)
        };
        if !on_bar(x) {
            continue;
        }
        let (mut left, mut right) = (x, x);
        while left > 0 && on_bar(left - 1) {
            left -= 1;
        }
        while right + 1 < width && on_bar(right + 1) {
            right += 1;
        }
        if right - left > 60 {
            return Ok(Some(CardHit {
                target: CardTarget::Scrub,
                fraction: (x - left) as f32 / (right - left) as f32,
            }));
        }
    }
    Ok(None)
}

/// Pure `_in_right_panel` threshold in physical pixels. A failed/zero DPI query uses 96 DPI.
pub fn point_in_right_panel(point: ScreenPoint, window_right: i32, dpi: u32) -> bool {
    f64::from(point.x)
        >= f64::from(window_right) - 340.0 * f64::from(if dpi == 0 { 96 } else { dpi }) / 96.0
}

/// RAII owner of one input thread. Drop signals, wakes and joins it; hotkeys leave with the thread.
pub struct InputWatcher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl InputWatcher {
    /// Stop and join this watcher without changing the caller's shared stop flag. Idempotent.
    pub fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            thread
                .join()
                .map_err(|_| Error::Protocol("input watcher panicked"))?;
        }
        Ok(())
    }
}

impl Drop for InputWatcher {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(any(windows, test))]
fn spawn_watcher(
    name: &str,
    external_stop: Arc<AtomicBool>,
    mut tick: impl FnMut() -> bool + Send + 'static,
) -> Result<InputWatcher> {
    let stop = Arc::new(AtomicBool::new(false));
    let local_stop = stop.clone();
    let thread = thread::Builder::new().name(name.into()).spawn(move || {
        while !local_stop.load(Ordering::Acquire) && !external_stop.load(Ordering::Acquire) {
            if !tick() {
                break;
            }
            thread::park_timeout(POLL);
        }
    })?;
    Ok(InputWatcher {
        stop,
        thread: Some(thread),
    })
}

/// Set per-monitor-v2 awareness before cursor/window queries. Already-aware hosts are accepted.
pub fn enable_dpi_awareness() -> Result<()> {
    #[cfg(windows)]
    {
        crate::win32::enable_dpi_awareness()
    }
    #[cfg(not(windows))]
    {
        crate::unsupported()
    }
}

/// Read the foreground process image basename, case-insensitive `t3 code` substring as in Python.
/// An inaccessible process or missing foreground window returns false.
pub fn foreground_is_t3() -> Result<bool> {
    #[cfg(windows)]
    {
        Ok(crate::win32::foreground_is_t3())
    }
    #[cfg(not(windows))]
    {
        crate::unsupported()
    }
}

/// Apply `_in_right_panel` to the foreground window's right edge and DPI.
pub fn in_right_panel(point: ScreenPoint) -> Result<bool> {
    #[cfg(windows)]
    {
        crate::win32::in_right_panel(point)
    }
    #[cfg(not(windows))]
    {
        let _ = point;
        crate::unsupported()
    }
}

/// Capture only a transient 1200x17 GDI strip, then run the pure detector at (600,8).
/// Caller must check T3 foreground/right panel first. Pixels are never persisted.
pub fn read_click(point: ScreenPoint, config: ClickConfig) -> Result<Option<CardHit>> {
    config.validate()?;
    #[cfg(windows)]
    {
        hit_test(
            &crate::win32::capture_strip(point)?,
            ScreenPoint { x: 600, y: 8 },
            config,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = point;
        crate::unsupported()
    }
}

/// Read CF_UNICODETEXT, always unlocking/closing it. Unavailable/non-text returns None.
/// The player, rather than this reader, limits link parsing to the first 4000 characters.
pub fn read_clipboard_text() -> Result<Option<String>> {
    #[cfg(windows)]
    {
        crate::win32::read_clipboard_text()
    }
    #[cfg(not(windows))]
    {
        crate::unsupported()
    }
}

/// Poll left-button rising edges every 12 ms; emit hits only in T3's right panel.
pub fn watch_clicks(
    config: ClickConfig,
    stop: Arc<AtomicBool>,
    events: Sender<InputEvent>,
) -> Result<InputWatcher> {
    config.validate()?;
    #[cfg(windows)]
    {
        let mut was_down = false;
        spawn_watcher("dt-clicks", stop, move || {
            let down = crate::win32::left_button_down();
            if down && !was_down {
                let check = || -> Result<Option<CardHit>> {
                    let point = crate::win32::cursor_position()?;
                    if foreground_is_t3()? && in_right_panel(point)? {
                        read_click(point, config)
                    } else {
                        Ok(None)
                    }
                };
                match check() {
                    Ok(Some(hit)) if events.send(InputEvent::Card(hit)).is_err() => return false,
                    Err(error) => eprintln!("click check failed: {error}"),
                    _ => {}
                }
            }
            was_down = down;
            true
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (stop, events);
        crate::unsupported()
    }
}

/// Register MOD_NOREPEAT media keys on the worker thread. Taken keys log and do not fail startup.
/// The message queue is polled, so stop/drop wakes immediately without cross-thread Win32 posts.
pub fn watch_media_keys(stop: Arc<AtomicBool>, events: Sender<InputEvent>) -> Result<InputWatcher> {
    #[cfg(windows)]
    {
        let mut keys = None;
        spawn_watcher("dt-media-keys", stop, move || {
            let keys = keys.get_or_insert_with(crate::win32::MediaKeys::register);
            keys.dispatch(&events)
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (stop, events);
        crate::unsupported()
    }
}

#[cfg(test)]
mod tests;
