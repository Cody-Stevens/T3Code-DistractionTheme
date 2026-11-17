//! Physical-pixel input from `watch_clicks`, `read_click`, `read_clipboard_text`, `watch_media_keys`.

use dt_core::{CardTarget, Result};
use std::sync::{Arc, atomic::AtomicBool, mpsc::Sender};

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

/// Port `watch_media_keys` and `card_action` inputs, delivered to the player command thread.
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

/// Port `GLOWTUBE_CLICK_FILL` and `read_click` thresholds.
#[derive(Clone, Copy, Debug)]
pub struct ClickConfig {
    /// Default .32 of a 29x17 solid marker area; button color tolerance is 10/channel.
    pub minimum_fill: f32,
    /// Progress-bar tolerance per RGB channel, default 14; require >60 contiguous pixels.
    pub bar_tolerance: u8,
}

/// RAII owner of a click watcher or media-key message loop from Python's watcher threads.
/// Implementation must unregister hotkeys, wake a blocked message loop and join its thread on drop.
pub struct InputWatcher {
    _private: (),
}

impl InputWatcher {
    /// Port watcher shutdown: signal termination, wake the Win32 loop, release resources and join.
    pub fn stop(&mut self) -> Result<()> {
        todo!("stop input watcher")
    }
}

/// Port `play`'s SetProcessDpiAwarenessContext(-4); call before screen/cursor/window queries.
/// Match physical pixels at 100% and 150% display scale.
pub fn enable_dpi_awareness() -> Result<()> {
    todo!("per-monitor-v2 DPI awareness")
}

/// Port `_foreground_is_t3`: query process image name of foreground window, case-insensitive
/// `t3 code` filename check. Failure to inspect another process returns false, not a click hit.
pub fn foreground_is_t3() -> Result<bool> {
    todo!("foreground process identity")
}

/// Port `_in_right_panel`: accept x >= foreground right - 340 * (window DPI / 96).
pub fn in_right_panel(_point: ScreenPoint) -> Result<bool> {
    todo!("DPI-scaled right-panel bounds")
}

/// Port `read_click`: GDI capture 1200x17 around the point, solid 29x17 marker test, then scrub
/// scan from central row outward. Discard pixels immediately; never write screenshots/log pixels.
/// Caller must first check T3 foreground/right panel. Targets: 28/28 at 100/150%, <=1/400 false hits.
pub fn read_click(_point: ScreenPoint, _config: ClickConfig) -> Result<Option<CardHit>> {
    todo!("transient pixel-strip marker recognition")
}

/// Port `read_clipboard_text`: CF_UNICODETEXT under OpenClipboard/GlobalLock, always unlock/close.
/// Return None for unavailable/non-text clipboard; player scans at most the first 4000 characters.
pub fn read_clipboard_text() -> Result<Option<String>> {
    todo!("read Unicode clipboard")
}

/// Port `watch_clicks`: poll left-button edges every 12 ms, check foreground and right panel,
/// emit only verified CardHit values. Stop when `stop` is true or receiver closes; own no player lock.
pub fn watch_clicks(
    _config: ClickConfig,
    _stop: Arc<AtomicBool>,
    _events: Sender<InputEvent>,
) -> Result<InputWatcher> {
    todo!("spawn click watcher")
}

/// Port `watch_media_keys`: MOD_NOREPEAT hotkeys for VK_MEDIA_PLAY_PAUSE/NEXT_TRACK/PREV_TRACK.
/// Report registration conflicts without stealing keys, release registrations on stop/drop.
pub fn watch_media_keys(
    _stop: Arc<AtomicBool>,
    _events: Sender<InputEvent>,
) -> Result<InputWatcher> {
    todo!("spawn media-key loop")
}
