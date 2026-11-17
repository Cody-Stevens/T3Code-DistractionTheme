//! Now Playing image from `card_image.py`. Marker RGB values live in dt-core for the input reader.

use dt_core::{Frame, Result};
use std::path::PathBuf;

/// Port `card_image.QueueItem.state`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueueState {
    /// Ready or unresolved, no status suffix.
    #[default]
    Normal,
    /// Download/analysis is in progress.
    Loading,
    /// Load failed; display a failed suffix.
    Failed,
}

/// Port `card_image.QueueItem`: display title and lazy-load state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    /// Track title, ellipsized to available text width.
    pub title: String,
    /// Loading/failure suffix state.
    pub state: QueueState,
}

/// Port `card_image.render` arguments; equality can key `Player.card`'s cached result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardContent {
    /// Current title, or `Nothing playing`.
    pub title: String,
    /// Artist/time or transient status line.
    pub subtitle: String,
    /// Draw pause bars when true, otherwise the play triangle.
    pub playing: bool,
    /// At most the first four entries are drawn.
    pub upcoming: Vec<QueueItem>,
}

/// Fonts supplied to the port of `card_image._font`; do not embed licensed system fonts.
#[derive(Clone, Debug)]
pub struct CardRenderOptions {
    /// Directory containing segoeui.ttf and seguisb.ttf; provide a deterministic fallback if absent.
    pub fonts_dir: PathBuf,
}

/// Port `card_image.Card`: WebP data plus CSS height.
#[derive(Clone, Debug)]
pub struct Card {
    /// Lossless WebP base64, <=9,000 ASCII characters, no data URL prefix.
    pub image: String,
    /// Height in CSS pixels; image is 276*2 pixels wide and height*2 pixels tall.
    pub height: u32,
}

/// Port `card_image.render`: draw title/subtitle, controls, four queue rows and copied-link buttons.
/// Leave spectrum/progress gaps for CSS; draw at 2x, quantize to 16 colors (32/64 fallback),
/// preserve each present marker EXACTLY, then lossless WebP. Return an error if >9 KB base64.
pub fn render(_content: &CardContent, _options: &CardRenderOptions) -> Result<Card> {
    todo!("port card_image.render")
}

/// Port `card_image._encode`: palette-reduce without dithering, verify all present marker colors
/// survive, then encode lossless RGB WebP. Decode in verification to assert pixel-exact markers.
/// Return base64 within `CARD_BASE64_LIMIT`; never send the card through a lossy codec.
pub fn encode(_image: &Frame) -> Result<String> {
    todo!("port card_image._encode")
}
