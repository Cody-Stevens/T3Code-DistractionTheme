//! Measured budgets and Python defaults. Durations use media time unless marked wall time.

use std::time::Duration;

/// Video frames per second (`glowtube.FPS`).
pub const FPS: u32 = 24;
/// Stereo PCM sample frames per second (`glowtube.SR`).
pub const SAMPLE_RATE: u32 = 48_000;
/// Channels in each interleaved PCM sample frame.
pub const AUDIO_CHANNELS: usize = 2;
/// Samples per channel in one video frame (exact at 48 kHz / 24 fps).
pub const SAMPLES_PER_FRAME: u32 = SAMPLE_RATE / FPS;
/// Decoded RGB24 width.
pub const FRAME_W: u32 = 640;
/// Decoded RGB24 height.
pub const FRAME_H: u32 = 360;
/// Player width in CSS pixels (`glowtube.VIDEO_W`).
pub const PLAYER_W: u32 = 800;
/// Player height in CSS pixels (`glowtube.VIDEO_H`).
pub const PLAYER_H: u32 = 450;
/// Glow bitmap width.
pub const GLOW_W: u32 = 128;
/// Glow bitmap height.
pub const GLOW_H: u32 = 72;
/// Glow size relative to the player (`theme_css.GLOW_SCALE`).
pub const GLOW_SCALE: u32 = 4;
/// Number of logarithmic spectrum bands.
pub const BANDS: usize = 16;
/// Theme write cadence; wall-time writes also obey `MIN_WRITE_GAP`.
pub const WRITE_INTERVAL: Duration = Duration::from_millis(250);
/// Frames between theme write starts (`glowtube.STEP`).
pub const WRITE_STEP_FRAMES: u32 = 6;
/// Frames in a normal write, including two spares (`glowtube.SPAN`).
pub const WRITE_SPAN_FRAMES: u32 = 8;
/// Minimum wall-time quiet interval after a write (`glowtube.MIN_GAP`).
pub const MIN_WRITE_GAP: Duration = Duration::from_millis(150);
/// T3's trailing theme-file debounce, not a safe write interval.
pub const THEME_DEBOUNCE: Duration = Duration::from_millis(100);
/// Maximum CSS Unicode characters in a generated write.
pub const CSS_LIMIT: usize = 49_500;
/// Server's absolute CSS character limit.
pub const T3_CSS_LIMIT: usize = 50_000;
/// Target median theme build time.
pub const BUILD_MEDIAN_BUDGET: Duration = Duration::from_millis(100);
/// Target worst theme build time.
pub const BUILD_WORST_BUDGET: Duration = Duration::from_millis(200);
/// Content duration in the long write before a favicon swap.
pub const QUIET_CONTENT: Duration = Duration::from_millis(2_500);
/// Wall time to suspend writes after that long write.
pub const QUIET_HOLD: Duration = Duration::from_millis(2_200);
/// Wall time after the long write before publishing the favicon.
pub const QUIET_SETTLE: Duration = Duration::from_millis(450);
/// Normal maximum favicon clip length, before the overlap.
pub const FAVICON_CLIP: Duration = Duration::from_secs(45);
/// First favicon clip length after start or seek; the next grows to 24 seconds.
pub const FAVICON_FIRST_CLIP: Duration = Duration::from_secs(8);
/// Lead time for encoding the first favicon clip.
pub const FAVICON_FIRST_LEAD: Duration = Duration::from_millis(4_500);
/// Adjacent favicon clips overlap by this much media time.
pub const FAVICON_OVERLAP: Duration = Duration::from_secs(1);
/// Default API-to-display latency (`GLOWTUBE_FAVICON_LATENCY` can override it).
pub const FAVICON_SHOW_LATENCY: Duration = Duration::from_millis(500);
/// Files above this size bypass T3's favicon thumbnail download.
pub const FAVICON_LOADER_LIMIT: usize = 4 * 1024 * 1024;
/// Minimum SVG size; padding must exceed the loader's 4 MiB limit.
pub const FAVICON_MIN_BYTES: usize = 4_300_000;
/// Maximum favicon data URL cached by the client.
pub const FAVICON_CACHE_CHARS: usize = 32_768;
/// Recent uniquely named clips retained for late client requests.
pub const FAVICON_KEEP: usize = 30;
/// Theme clips encoded five writes (1.25 seconds) ahead.
pub const AHEAD_WRITES: usize = 5;
/// Independent short-clip encode workers; favicon encoding has its own worker.
pub const AHEAD_WORKERS: usize = 6;
/// Base64 room reserved for changing glow and card content.
pub const AHEAD_SLACK_CHARS: usize = 600;
/// Helper's loopback-only control port.
pub const CONTROL_PORT: u16 = 47_873;
/// Python's HTTP control timeout.
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
/// Theme file stem and selected theme ID.
pub const THEME_ID: &str = "glowtube";
/// T3 project title and theme display name.
pub const DISPLAY_NAME: &str = "GlowTube";
/// Card width in CSS pixels.
pub const CARD_WIDTH: u32 = 276;
/// Card raster scale relative to CSS pixels.
pub const CARD_SCALE: u32 = 2;
/// Maximum card WebP base64 characters (9 KB, decimal).
pub const CARD_BASE64_LIMIT: usize = 9_000;
/// Maximum queue rows visible in the card.
pub const CARD_QUEUE_ROWS: usize = 4;
/// Card outer padding in CSS pixels.
pub const CARD_PAD: u32 = 12;
/// Spectrum top and bottom in CSS pixels.
pub const CARD_SPECTRUM_Y: (u32, u32) = (50, 94);
/// Center of playback controls in CSS pixels.
pub const CARD_CONTROL_Y: u32 = 118;
/// Progress bar left edge and height in CSS pixels.
pub const CARD_BAR: (u32, u32) = (50, 6);
/// Queue row height and gap in CSS pixels.
pub const CARD_ROW: (u32, u32) = (24, 3);
/// Exact RGB marker colors: toggle, play link, queue link, rows 0..3.
pub const CARD_MARKS: [[u8; 3]; 7] = [
    [0x58, 0x18, 0x38],
    [0x18, 0x58, 0x38],
    [0x38, 0x18, 0x58],
    [0x58, 0x38, 0x18],
    [0x18, 0x38, 0x58],
    [0x38, 0x58, 0x18],
    [0x58, 0x58, 0x18],
];
/// Unplayed progress color, drawn in CSS and recognized by the pixel reader.
pub const MARK_TRACK: [u8; 3] = [0x4a, 0x4a, 0x66];
/// Played progress color, drawn in CSS and recognized by the pixel reader.
pub const MARK_FILL: [u8; 3] = [0xe6, 0xe6, 0xff];
/// Default gain; the player clamps commands to 0..=1.5.
pub const DEFAULT_VOLUME: f32 = 0.8;
/// Default visual intensity, also the maximum (0..=11).
pub const DEFAULT_CHAOS: f32 = 11.0;
/// Estimated theme write-to-display delay.
pub const DEFAULT_VIDEO_OFFSET: Duration = Duration::from_millis(160);
/// Supervisor poll interval.
pub const SERVICE_POLL: Duration = Duration::from_secs(3);
