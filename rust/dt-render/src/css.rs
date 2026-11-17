//! Self-contained theme writes from `theme_css.py` and `glowtube.build_dynamic`.

use crate::{
    card::Card,
    encode::{AvifEncoder, EncodedVideo},
};
use dt_core::{Frame, Result};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

/// Port `Player.batch`'s sparse keyframe dictionaries: b0..b15, bs, kk, pr, c0..c3 and aa.
/// Values are CSS numbers or #rrggbb; validate names/values and reject backslashes.
pub type CssValues = BTreeMap<String, String>;

/// Cached quality ladder and sequence counter from `glowtube.SEQ/LEVEL/QUALITY`.
/// Only the write-loop owner mutates this; ahead workers return independent encoded results.
#[derive(Debug, Default)]
pub struct DynamicState {
    /// Unique animation sequence, modulo 100000. Every write must restart animations by name.
    pub sequence: u32,
    /// Previous fallback ladder index (initialize to 4 when implementing the builder).
    pub level: Option<usize>,
    /// Human-readable quality string exported by the control server.
    pub quality: String,
}

/// Port `glowtube.build_dynamic` arguments; frames, glows and values are aligned and nonempty.
pub struct DynamicInput<'a> {
    /// Usually eight 640x360 frames, one when paused, or 60 during a quiet-window write.
    pub frames: &'a [Arc<Frame>],
    /// Matching pre-rendered 128x72 glow frames (empty disables glow).
    pub glows: &'a [Arc<Frame>],
    /// Sparse per-frame values: bass/kick each frame, bands every second, colors/progress every sixth.
    pub values: &'a [CssValues],
    /// Cached card (the CSS still carries it on every write).
    pub card: &'a Card,
    /// Whether animation advances instead of applying just the first frame's values.
    pub playing: bool,
    /// Intensity in 0..=11.
    pub chaos: f32,
    /// Ahead result, rechecked against this write's room before use.
    pub prepared: Option<&'a EncodedVideo>,
    /// Favicon currently covers the player; use cheap hidden-video fallback.
    pub cheap: bool,
}

/// Build result of `glowtube.build_dynamic`, including the next ahead-worker size budget.
#[derive(Clone, Debug)]
pub struct BuiltTheme {
    /// Complete static+dynamic+card+glow+video CSS, <=49,500 Unicode characters.
    pub css: String,
    /// Base64 characters available to upcoming video encodes, before 600-character slack.
    pub video_room: usize,
}

const CONV: &str = "[data-workspace-area=\"conversation\"]";
const CARD: &str = "[data-thread-details-card]";
const ANIMATED: &str = "[data-workspace-area=\"conversation\"],[data-workspace-area=\"conversation\"]::before,[data-workspace-area=\"conversation\"]::after,[data-slot=\"composer-host\"],[data-chat-header],[data-slot=\"sidebar-inner\"],[data-thread-item],[data-thread-details-card],[data-thread-details-card]::before,[data-thread-details-card]::after";

/// Current Python gtjump/player-band block, isolated for the pending text-flow re-sync.
pub fn text_flow_css() -> &'static str {
    include_str!("text_flow.css")
}

/// Complete static geometry, properties, live layers, anchors and view timelines.
pub fn static_css(video_w: u32, video_h: u32) -> String {
    include_str!("static.css")
        .replace("{VIDEO_W}", &video_w.to_string())
        .replace("{VIDEO_H}", &video_h.to_string())
        .replace("{TEXT_FLOW}", text_flow_css())
}
/// Card height in CSS pixels.
pub fn card_height_css(height: u32) -> String {
    format!("{CARD}{{--cardh:{height}px}}")
}
/// Quoted lossless WebP data URL. Invalid base64 is replaced with an empty image.
pub fn card_image_css(card: &Card) -> String {
    format!(
        "{CARD}::before{{--cardimg:url(\"data:image/webp;base64,{}\")}}",
        safe_data(&card.image)
    )
}
/// Quoted animated AVIF data URL on both glow consumers.
pub fn glow_css(base64: &str) -> String {
    format!(
        "{CONV},{CONV}::before{{--glow:url(\"data:image/avif;base64,{}\")}}",
        safe_data(base64)
    )
}
/// Raise the current favicon deck over the other deck.
pub fn favicon_front_css(deck: char) -> Result<String> {
    if !matches!(deck, 'a' | 'b') {
        return Err(dt_core::Error::InvalidInput("favicon deck must be a or b"));
    }
    Ok(format!(
        "img[src*=\"glowtube-clip-{deck}-\"]{{z-index:2147483001!important}}"
    ))
}
fn safe_data(value: &str) -> &str {
    if valid_data(value) { value } else { "" }
}
fn valid_data(value: &str) -> bool {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .is_ok()
}
fn valid_name(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
fn declarations(values: &CssValues) -> Result<String> {
    let mut out = Vec::new();
    for (key, value) in values {
        let color = matches!(key.as_str(), "c0" | "c1" | "c2" | "c3" | "aa");
        let number =
            matches!(key.as_str(), "bs" | "kk" | "pr") || (0..16).any(|i| key == &format!("b{i}"));
        let valid = if color {
            value.len() == 7
                && value.starts_with('#')
                && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        } else {
            number
                && value.parse::<f64>().is_ok_and(f64::is_finite)
                && value
                    .bytes()
                    .all(|c| c.is_ascii_digit() || b"+-.eE".contains(&c))
        };
        if !valid {
            return Err(dt_core::Error::InvalidInput(
                "invalid CSS variable or value",
            ));
        }
        out.push(format!("--{key}:{value}"));
    }
    Ok(out.join(";"))
}
/// Sparse evenly spaced keyframes; empty maps are omitted.
pub fn keyframe_block(name: &str, frames: &[CssValues]) -> Result<String> {
    if !valid_name(name) {
        return Err(dt_core::Error::InvalidInput("invalid animation name"));
    }
    let mut out = format!("@keyframes {name}{{");
    for (i, values) in frames.iter().enumerate() {
        if values.is_empty() {
            continue;
        }
        let pct = if frames.len() == 1 {
            0.0
        } else {
            i as f64 * 100.0 / (frames.len() - 1) as f64
        };
        out.push_str(&format!("{pct:.2}%{{{}}}", declarations(values)?));
    }
    out.push('}');
    Ok(out)
}
/// Linear keyframes on consumers only, with forwards or infinite fill.
pub fn animate(name: &str, duration: Duration, infinite: bool) -> Result<String> {
    if !valid_name(name) || duration.is_zero() {
        return Err(dt_core::Error::InvalidInput("invalid animation"));
    }
    Ok(format!(
        "{ANIMATED}{{animation:{name} {:.3}s linear {}}}",
        duration.as_secs_f64(),
        if infinite { "infinite" } else { "forwards" }
    ))
}
/// Paused values applied only to consumers of animated properties.
pub fn static_values(values: &CssValues) -> Result<String> {
    Ok(format!("{ANIMATED}{{{}}}", declarations(values)?))
}
/// Python's fixed 16-second drifting idle palette.
pub fn idle_css() -> String {
    include_str!("idle.css").into()
}
fn complete(css: String) -> Result<String> {
    if css.contains('\\') || css.chars().count() > dt_core::constants::CSS_LIMIT {
        return Err(dt_core::Error::InvalidInput(
            "CSS exceeds renderer constraints",
        ));
    }
    Ok(css)
}
fn checked_card(card: &Card) -> Result<()> {
    if card.image.len() > dt_core::constants::CARD_BASE64_LIMIT || !valid_data(&card.image) {
        return Err(dt_core::Error::InvalidInput("invalid card image"));
    }
    Ok(())
}
/// Standalone idle write, including the cached card.
pub fn idle_theme(card: &Card) -> Result<String> {
    checked_card(card)?;
    complete(
        static_css(800, 450) + &idle_css() + &card_height_css(card.height) + &card_image_css(card),
    )
}
/// Build every layer; reuse fitting ahead video or descend the synchronous quality ladder.
pub fn build_dynamic(
    encoder: &dyn AvifEncoder,
    state: &mut DynamicState,
    input: DynamicInput<'_>,
) -> Result<BuiltTheme> {
    use crate::encode::{LADDER, avif, encode_video};
    use dt_core::{Error, constants::CSS_LIMIT};
    let n = input.frames.len();
    if n == 0
        || input.values.len() != n
        || (!input.glows.is_empty() && input.glows.len() != n)
        || !input.chaos.is_finite()
        || !(0.0..=11.0).contains(&input.chaos)
    {
        return Err(Error::InvalidInput(
            "unaligned dynamic input or invalid intensity",
        ));
    }
    checked_card(input.card)?;
    state.sequence = (state.sequence % 100000 + 1) % 100000;
    let name = format!("d{}", state.sequence);
    let vars = if input.playing && n > 1 {
        keyframe_block(&name, input.values)?
            + &animate(&name, Duration::from_secs_f64(n as f64 / 24.0), false)?
    } else {
        static_values(&input.values[0])?
    };
    let mut head = format!(":root{{--g:{:.2}}}{vars}", input.chaos / 11.0)
        + &card_height_css(input.card.height)
        + &card_image_css(input.card);
    if !input.glows.is_empty() {
        head.push_str(&glow_css(&avif(encoder, input.glows, 30)?));
    }
    let prefix = static_css(800, 450) + &head;
    const WRAPPER: &str = ":root{--vid:url(\"data:image/avif;base64,\")}";
    let room = CSS_LIMIT
        .checked_sub(prefix.chars().count() + WRAPPER.len())
        .filter(|v| *v > 0)
        .ok_or(Error::InvalidInput("no CSS room for video"))?;
    let mut quality = String::new();
    let data = if let Some(prepared) = input
        .prepared
        .filter(|p| p.base64.len() <= room && valid_data(&p.base64))
    {
        quality = format!(
            "{}x{} 24fps q{} (ahead, speed 6)",
            prepared.width,
            prepared.width * 9 / 16,
            prepared.quality
        );
        prepared.base64.clone()
    } else {
        let mut level = if input.cheap {
            LADDER.len() - 3
        } else {
            state.level.unwrap_or(4).min(LADDER.len() - 1)
        };
        let mut encoded = encode_video(encoder, input.frames, level)?;
        while encoded.base64.len() > room && level < LADDER.len() - 1 {
            level += 1;
            encoded = encode_video(encoder, input.frames, level)?;
        }
        if encoded.base64.len() > room {
            level = LADDER.len() - 1;
            encoded = encode_video(encoder, &input.frames[..1], level)?;
        }
        if encoded.base64.len() > room {
            return Err(Error::InvalidInput("video cannot fit CSS budget"));
        }
        if !input.cheap {
            // Retain the known fitting fallback. Repeated upward probes cost another full
            // synchronous encode; quality exploration belongs to encode_to_fit ahead workers.
            state.level = Some(level);
        }
        quality.push_str(&format!(
            "{}x{} 24fps q{} (now, speed 10)",
            encoded.width,
            encoded.width * 9 / 16,
            encoded.quality
        ));
        encoded.base64
    };
    state.quality = quality;
    Ok(BuiltTheme {
        css: complete(prefix + &format!(":root{{--vid:url(\"data:image/avif;base64,{data}\")}}"))?,
        video_room: room,
    })
}
