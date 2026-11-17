//! Now Playing image from `card_image.py`. Marker RGB values live in dt-core for the input reader.

use dt_core::{Frame, Result};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

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

use dt_core::{
    Error,
    constants::{CARD_BASE64_LIMIT, CARD_MARKS},
};
use fontdue::{Font, FontSettings};
use image::{Rgb, RgbImage};
const BG: [u8; 3] = [17, 10, 29];
struct Face(Option<Arc<Font>>);
impl Face {
    fn load(path: &std::path::Path) -> Self {
        // Card subtitles change every write; keep parsed faces across those renders.
        type Fonts = Vec<(PathBuf, Arc<Font>)>;
        static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
        let cache = FONTS.get_or_init(|| Mutex::new(Vec::new()));
        if let Some((_, font)) = cache
            .lock()
            .expect("font cache")
            .iter()
            .find(|(p, _)| p == path)
        {
            return Self(Some(font.clone()));
        }
        let font = std::fs::read(path)
            .ok()
            .and_then(|b| Font::from_bytes(b, FontSettings::default()).ok())
            .map(Arc::new);
        if let Some(font) = &font {
            let mut cache = cache.lock().expect("font cache");
            if cache.len() >= 4 {
                cache.remove(0);
            }
            cache.push((path.to_owned(), font.clone()));
        }
        Self(font)
    }
    fn width(&self, text: &str, size: f32) -> f32 {
        match &self.0 {
            Some(f) => text.chars().map(|c| f.metrics(c, size).advance_width).sum(),
            None => text.chars().count() as f32 * size * 0.65,
        }
    }
    fn fit(&self, text: &str, size: f32, width: f32) -> String {
        let text = text.replace(['\r', '\n', '\t'], " ");
        if self.width(&text, size) <= width {
            return text;
        }
        let mut out = text;
        while !out.is_empty() && self.width(&(out.clone() + "\u{2026}"), size) > width {
            out.pop();
        }
        out.trim_end().to_owned() + "\u{2026}"
    }
    fn draw(&self, img: &mut RgbImage, text: &str, x: f32, y: f32, size: f32, color: [u8; 3]) {
        let mut x = x;
        if let Some(font) = &self.0 {
            let baseline = y + font
                .horizontal_line_metrics(size)
                .map_or(size, |m| m.ascent);
            for ch in text.chars() {
                let (m, coverage) = font.rasterize(ch, size);
                for gy in 0..m.height {
                    for gx in 0..m.width {
                        let px = x as i32 + m.xmin + gx as i32;
                        let py = baseline as i32 - m.ymin - m.height as i32 + gy as i32;
                        if px >= 0 && py >= 0 && px < img.width() as i32 && py < img.height() as i32
                        {
                            let a = u32::from(coverage[gy * m.width + gx]);
                            let bg = img.get_pixel(px as u32, py as u32).0;
                            img.put_pixel(
                                px as u32,
                                py as u32,
                                Rgb(std::array::from_fn(|c| {
                                    ((u32::from(color[c]) * a + u32::from(bg[c]) * (255 - a) + 127)
                                        / 255) as u8
                                })),
                            );
                        }
                    }
                }
                x += m.advance_width;
            }
        } else {
            use font8x8::UnicodeFonts;
            let scale = (size / 8.0).floor().max(1.0) as u32;
            for ch in text.chars() {
                if let Some(rows) = font8x8::BASIC_FONTS
                    .get(ch)
                    .or_else(|| font8x8::BASIC_FONTS.get('?'))
                {
                    for (dy, row) in rows.into_iter().enumerate() {
                        for dx in 0..8 {
                            if row & (1 << dx) != 0 {
                                for sy in 0..scale {
                                    for sx in 0..scale {
                                        let px = x as u32 + dx * scale + sx;
                                        let py = y as u32 + dy as u32 * scale + sy;
                                        if px < img.width() && py < img.height() {
                                            img.put_pixel(px, py, Rgb(color));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                x += size * 0.65;
            }
        }
    }
}
fn rounded(img: &mut RgbImage, x: f32, y: f32, w: f32, h: f32, r: f32, color: [u8; 3]) {
    let (x, y, w, h, r) = (x * 2.0, y * 2.0, w * 2.0, h * 2.0, r * 2.0);
    for py in y as u32..=(y + h) as u32 {
        for px in x as u32..=(x + w) as u32 {
            let nx = (px as f32).clamp(x + r, x + w - r);
            let ny = (py as f32).clamp(y + r, y + h - r);
            if (px as f32 - nx).powi(2) + (py as f32 - ny).powi(2) <= r * r
                && px < img.width()
                && py < img.height()
            {
                img.put_pixel(px, py, Rgb(color));
            }
        }
    }
}
fn triangle(img: &mut RgbImage, x: f32, y: f32, w: f32, h: f32) {
    for py in (y * 2.0) as u32..=((y + h) * 2.0) as u32 {
        for px in (x * 2.0) as u32..=((x + w) * 2.0) as u32 {
            let fraction = 1.0 - ((py as f32 / 2.0 - (y + h / 2.0)) / (h / 2.0)).abs();
            if px as f32 / 2.0 <= x + w * fraction && px < img.width() && py < img.height() {
                img.put_pixel(px, py, Rgb([255; 3]));
            }
        }
    }
}
/// Draw the reference layout at 2x; fonts come from the supplied directory, with a bitmap fallback.
pub fn render(content: &CardContent, options: &CardRenderOptions) -> Result<Card> {
    let title = Face::load(&options.fonts_dir.join("seguisb.ttf"));
    let body = Face::load(&options.fonts_dir.join("segoeui.ttf"));
    let rows = &content.upcoming[..content.upcoming.len().min(4)];
    let buttons = 158 + rows.len().max(1) as u32 * 27 + 6;
    let height = buttons + 40;
    let mut img = RgbImage::from_pixel(552, height * 2, Rgb(BG));
    title.draw(
        &mut img,
        &title.fit(&content.title, 26.0, 504.0),
        24.0,
        18.0,
        26.0,
        [255; 3],
    );
    body.draw(
        &mut img,
        &body.fit(&content.subtitle, 23.0, 504.0),
        24.0,
        58.0,
        23.0,
        [201, 195, 221],
    );
    rounded(&mut img, 12.0, 104.0, 28.0, 28.0, 14.0, CARD_MARKS[0]);
    if content.playing {
        for x in [21.0, 27.5] {
            rounded(&mut img, x, 112.0, 3.5, 12.0, 1.0, [255; 3]);
        }
    } else {
        triangle(&mut img, 22.0, 111.0, 11.0, 14.0);
    }
    title.draw(&mut img, "UP NEXT", 24.0, 280.0, 21.0, [154, 147, 184]);
    if rows.is_empty() {
        body.draw(
            &mut img,
            "Nothing queued",
            24.0,
            324.0,
            23.0,
            [138, 132, 166],
        );
    }
    for (i, item) in rows.iter().enumerate() {
        let top = 158 + i as u32 * 27;
        rounded(
            &mut img,
            12.0,
            top as f32,
            252.0,
            24.0,
            6.0,
            CARD_MARKS[3 + i],
        );
        let suffix = match item.state {
            QueueState::Normal => "",
            QueueState::Loading => "  \u{00b7} loading",
            QueueState::Failed => "  \u{00b7} failed",
        };
        let label = body.fit(
            &format!("{}.  {}", i + 1, item.title),
            23.0,
            472.0 - body.width(suffix, 23.0),
        ) + suffix;
        body.draw(
            &mut img,
            &label,
            40.0,
            (top + 4) as f32 * 2.0,
            23.0,
            [255; 3],
        );
    }
    for (left, mark, label) in [
        (12.0, 1, "Play copied link"),
        (142.0, 2, "Queue copied link"),
    ] {
        rounded(
            &mut img,
            left,
            buttons as f32,
            122.0,
            28.0,
            8.0,
            CARD_MARKS[mark],
        );
        let w = title.width(label, 21.0) + 26.0;
        let x0 = (left + 61.0) * 2.0 - w / 2.0;
        if mark == 1 {
            triangle(&mut img, x0 / 2.0, buttons as f32 + 9.5, 8.0, 9.0);
        } else {
            rounded(
                &mut img,
                x0 / 2.0,
                buttons as f32 + 13.0,
                8.0,
                2.0,
                0.0,
                [255; 3],
            );
            rounded(
                &mut img,
                x0 / 2.0 + 3.0,
                buttons as f32 + 10.0,
                2.0,
                8.0,
                0.0,
                [255; 3],
            );
        }
        title.draw(
            &mut img,
            label,
            x0 + 26.0,
            (buttons + 7) as f32 * 2.0,
            21.0,
            [255; 3],
        );
    }
    Ok(Card {
        image: encode(&Frame::new(552, height * 2, img.into_raw())?)?,
        height,
    })
}
fn palette(frame: &Frame, colors: usize) -> Vec<[u8; 3]> {
    use std::collections::BTreeMap;
    let mut histogram = BTreeMap::<[u8; 3], usize>::new();
    for p in frame.pixels().as_chunks::<3>().0.iter() {
        *histogram.entry([p[0], p[1], p[2]]).or_default() += 1;
    }
    let mut boxes = vec![histogram.into_iter().collect::<Vec<_>>()];
    // Pillow median cut prioritizes population and weights channel spans by luminance.
    while boxes.len() < colors {
        let choice = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .max_by_key(|(_, b)| b.iter().map(|(_, n)| n).sum::<usize>())
            .map(|(i, _)| i);
        let Some(i) = choice else {
            break;
        };
        let mut b = boxes.remove(i);
        let axis = (0..3)
            .max_by_key(|&c| {
                usize::from(
                    b.iter().map(|(p, _)| p[c]).max().unwrap_or(0)
                        - b.iter().map(|(p, _)| p[c]).min().unwrap_or(0),
                ) * [77, 150, 29][c]
            })
            .unwrap_or(0);
        b.sort_by_key(|(p, _)| p[axis]);
        let total = b.iter().map(|(_, n)| n).sum::<usize>();
        let mut sum = 0;
        let mut split = 0;
        for (j, (_, n)) in b.iter().enumerate() {
            sum += n;
            split = j + 1;
            if sum * 2 > total {
                break;
            }
        }
        while split < b.len() && b[split].0[axis] == b[split - 1].0[axis] {
            split += 1;
        }
        if split == b.len() {
            split = b.len() - 1;
            while split > 0 && b[split - 1].0[axis] == b[b.len() - 1].0[axis] {
                split -= 1;
            }
        }
        if split == 0 {
            split = b.len() / 2;
        }
        let right = b.split_off(split);
        boxes.push(b);
        boxes.push(right);
    }
    let mut palette: Vec<[u8; 3]> = boxes
        .into_iter()
        .map(|b| {
            let n = b.iter().map(|(_, n)| n).sum::<usize>();
            std::array::from_fn(|c| {
                ((b.iter().map(|(p, n)| usize::from(p[c]) * n).sum::<usize>() + n / 2) / n) as u8
            })
        })
        .collect();
    // Reserve distinct entries for present click markers, even if a small marker shares a box.
    let mut locked = vec![false; palette.len()];
    for mark in CARD_MARKS {
        if frame.pixels().as_chunks::<3>().0.iter().any(|p| p == &mark) {
            let index = palette
                .iter()
                .enumerate()
                .filter(|(i, _)| !locked[*i])
                .min_by_key(|(_, p)| {
                    (0..3)
                        .map(|c| (i32::from(p[c]) - i32::from(mark[c])).pow(2))
                        .sum::<i32>()
                })
                .map(|(i, _)| i)
                .expect("palette has room for all markers");
            palette[index] = mark;
            locked[index] = true;
        }
    }
    palette
}
/// Palette reduction locks every marker pixel before lossless WebP encoding.
pub fn encode(frame: &Frame) -> Result<String> {
    use base64::Engine;
    for colors in [16, 32, 64] {
        let palette = palette(frame, colors);
        let mut pixels = Vec::with_capacity(frame.pixels().len());
        let mut cache = std::collections::HashMap::<[u8; 3], [u8; 3]>::new();
        for p in frame.pixels().as_chunks::<3>().0.iter() {
            let key = [p[0], p[1], p[2]];
            let target = *cache.entry(key).or_insert_with(|| {
                *palette
                    .iter()
                    .min_by_key(|c| {
                        (0..3)
                            .map(|i| (i32::from(key[i]) - i32::from(c[i])).pow(2))
                            .sum::<i32>()
                    })
                    .expect("nonempty palette")
            });
            pixels.extend(target);
        }
        let mut config =
            webp::WebPConfig::new().map_err(|_| Error::Protocol("WebP configuration failed"))?;
        config.lossless = 1;
        config.quality = 100.0;
        // Method 6 took ~450 ms per changing card; method 3 takes ~12 ms and remains
        // lossless. The same palette/size checks below still enforce the card budget.
        config.method = 3;
        let encoded = webp::Encoder::from_rgb(&pixels, frame.width(), frame.height())
            .encode_advanced(&config)
            .map_err(|_| Error::Protocol("lossless WebP encode failed"))?;
        let data = encoded.to_vec();
        let result = base64::engine::general_purpose::STANDARD.encode(data);
        if result.len() <= CARD_BASE64_LIMIT {
            return Ok(result);
        }
    }
    Err(Error::InvalidInput("card exceeds base64 budget"))
}

#[cfg(test)]
mod tests {
    use super::Face;

    #[test]
    fn truncation_preserves_unicode_and_uses_one_ellipsis() {
        let font = Face(None);
        assert_eq!(
            font.fit("\u{1f3b5}abcdef", 10.0, 26.0),
            "\u{1f3b5}ab\u{2026}"
        );
        assert_eq!(font.fit("short", 10.0, 50.0), "short");
    }
}
