//! Pixels and CSS from the measured Python renderer. No T3 state or device ownership.
pub mod card;
pub mod css;
pub mod encode;

use dt_core::{AudioFeatureFrame, Error, Frame, Result};
use image::{RgbImage, imageops};

/// Boost HSV saturation and value, preserving hue, as in `glowtube.light`.
pub fn light(rgb: [f32; 3]) -> String {
    let c = rgb.map(|x| f64::from(x.clamp(0.0, 255.0)) / 255.0);
    let max = c.into_iter().fold(0.0_f64, f64::max);
    let min = c.into_iter().fold(1.0_f64, f64::min);
    let delta = max - min;
    let h = if delta == 0.0 {
        0.0
    } else if max == c[0] {
        ((c[1] - c[2]) / delta).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / delta + 2.0
    } else {
        (c[0] - c[1]) / delta + 4.0
    };
    let s = if max == 0.0 {
        0.0
    } else {
        (delta / max * 1.2).min(1.0)
    };
    let v = (max * 1.1 + 0.08).min(1.0);
    let chroma = v * s;
    let x = chroma * (1.0 - (h % 2.0 - 1.0).abs());
    let a = match h as u8 {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    let [r, g, b] = a.map(|x| ((x + v - chroma) * 255.0) as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Top fifth, right sixth, bottom fifth, left sixth, and whole-frame light colors.
pub fn edge_colors(frame: &Frame) -> Result<[String; 5]> {
    let (w, h) = (frame.width(), frame.height());
    if w < 6 || h < 5 {
        return Err(Error::InvalidInput(
            "edge strips require at least 6x5 pixels",
        ));
    }
    let regions = [
        (0, 0, w, h / 5),
        (w - w / 6, 0, w, h),
        (0, h - h / 5, w, h),
        (0, 0, w / 6, h),
        (0, 0, w, h),
    ];
    Ok(regions.map(|(x0, y0, x1, y1)| {
        let mut sum = [0_u64; 3];
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * w + x) * 3) as usize;
                for (c, value) in sum.iter_mut().enumerate() {
                    *value += u64::from(frame.pixels()[i + c]);
                }
            }
        }
        light(sum.map(|x| (x as f64 / f64::from((x1 - x0) * (y1 - y0))) as f32))
    }))
}

pub(crate) fn bitmap(frame: &Frame) -> RgbImage {
    // Frame validates this exact shape on construction.
    RgbImage::from_raw(frame.width(), frame.height(), frame.pixels().to_vec())
        .expect("validated RGB frame")
}

pub(crate) fn resized(frame: &Frame, width: u32, height: u32) -> Result<Frame> {
    if frame.width() == width && frame.height() == height {
        return Ok(frame.clone());
    }
    Frame::new(
        width,
        height,
        imageops::resize(
            &bitmap(frame),
            width,
            height,
            imageops::FilterType::Triangle,
        )
        .into_raw(),
    )
}

/// Low-resolution bass spill, kick flash and traveling ring from `glowtube.glow_frame`.
pub fn glow_frame(frame: &Frame, features: AudioFeatureFrame) -> Result<Frame> {
    let AudioFeatureFrame {
        bass, kick, ring, ..
    } = features;
    if [bass, kick]
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || !ring.is_finite()
        || ring < 0.0
    {
        return Err(Error::InvalidInput("invalid glow features"));
    }
    let spill = 1.35 + bass * 1.3 + kick * 0.5;
    let sw = (32.0 * spill).round_ties_even() as u32;
    let sh = (18.0 * spill).round_ties_even() as u32;
    let mut canvas = RgbImage::new(128, 72);
    // Area sampling matches Pillow BOX downsampling, without sharpening the light source.
    for y in 0..sh {
        for x in 0..sw {
            let x0 = (f64::from(x) * f64::from(frame.width()) / f64::from(sw)).round() as u32;
            let x1 = (f64::from(x + 1) * f64::from(frame.width()) / f64::from(sw)).round() as u32;
            let y0 = (f64::from(y) * f64::from(frame.height()) / f64::from(sh)).round() as u32;
            let y1 = (f64::from(y + 1) * f64::from(frame.height()) / f64::from(sh)).round() as u32;
            let x0 = x0.min(frame.width() - 1);
            let x1 = x1.max(x0 + 1).min(frame.width());
            let y0 = y0.min(frame.height() - 1);
            let y1 = y1.max(y0 + 1).min(frame.height());
            let mut sum = [0_u64; 3];
            let mut n = 0;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s +=
                            u64::from(frame.pixels()[((sy * frame.width() + sx) * 3) as usize + c]);
                    }
                    n += 1;
                }
            }
            if n > 0 {
                canvas.put_pixel(
                    x + (128 - sw) / 2,
                    y + (72 - sh) / 2,
                    image::Rgb(sum.map(|v| ((v + n / 2) / n) as u8)),
                );
            }
        }
    }
    let blurred = gaussian(&canvas, 2.5 + bass * 5.0 + kick * 2.0);
    let mut avg = [0.0_f32; 3];
    for px in frame.pixels().as_chunks::<3>().0.iter() {
        for c in 0..3 {
            avg[c] += f32::from(px[c]);
        }
    }
    let max = avg
        .into_iter()
        .fold(0.0_f32, f32::max)
        .max((frame.width() * frame.height()) as f32);
    avg = avg.map(|v| v / max * 255.0);
    let mut pixels = Vec::with_capacity(128 * 72 * 3);
    for (x, y, px) in blurred.enumerate_pixels() {
        let shell = if kick > 0.05 {
            (-((((x as f32 - 63.5).hypot(y as f32 - 35.5) - ring / 25.0) / 1.3).powi(2))).exp()
                * kick
        } else {
            0.0
        };
        let mut color = std::array::from_fn::<_, 3, _>(|c| {
            f32::from(px[c]) * (1.8 + bass * 3.5 + kick * 3.0) + shell * avg[c]
        });
        let lum = color.iter().sum::<f32>() / 3.0;
        color = color.map(|v| (lum + (v - lum) * 1.5).max(0.0));
        let scale = (255.0 / color.into_iter().fold(0.001_f32, f32::max)).min(1.0);
        pixels.extend(color.map(|v| (v * scale) as u8));
    }
    Frame::new(128, 72, pixels)
}

// Three fractional box passes per axis, matching Pillow GaussianBlur's fixed-point weights.
fn gaussian(source: &RgbImage, sigma: f32) -> RgbImage {
    let sigma2 = sigma * sigma / 3.0;
    let length = (12.0 * sigma2 + 1.0).sqrt();
    let l = ((length - 1.0) / 2.0).floor();
    let a =
        (2.0 * l + 1.0) * (l * (l + 1.0) - 3.0 * sigma2) / (6.0 * (sigma2 - (l + 1.0) * (l + 1.0)));
    let radius = l + a;
    let r = radius as i32;
    let ww = ((1_u32 << 24) as f32 / (radius * 2.0 + 1.0)) as u32;
    let fw = ((1_u32 << 24) - (2 * r as u32 + 1) * ww) / 2;
    let (w, h) = (source.width() as usize, source.height() as usize);
    let mut data = source.as_raw().clone();
    let mut out = vec![0; data.len()];
    for vertical in [false, true] {
        for _ in 0..3 {
            let (lines, len) = if vertical { (w, h) } else { (h, w) };
            for line in 0..lines {
                for c in 0..3 {
                    let index = |p: i32| {
                        let p = p.clamp(0, len as i32 - 1) as usize;
                        if vertical {
                            (p * w + line) * 3 + c
                        } else {
                            (line * w + p) * 3 + c
                        }
                    };
                    let mut sum = (-r..=r).map(|x| u32::from(data[index(x)])).sum::<u32>();
                    for x in 0..len as i32 {
                        let value = sum * ww
                            + (u32::from(data[index(x - r - 1)])
                                + u32::from(data[index(x + r + 1)]))
                                * fw;
                        out[index(x)] = ((value + (1 << 23)) >> 24) as u8;
                        sum =
                            sum + u32::from(data[index(x + r + 1)]) - u32::from(data[index(x - r)]);
                    }
                }
            }
            std::mem::swap(&mut data, &mut out);
        }
    }
    RgbImage::from_raw(w as u32, h as u32, data).expect("fixed glow shape")
}
