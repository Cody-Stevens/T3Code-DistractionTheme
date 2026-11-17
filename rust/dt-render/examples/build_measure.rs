//! End-to-end, offline write timing with real codecs and a cached card.
use dt_core::{AudioFeatureFrame, Frame};
use dt_render::{
    card::{self, CardContent, CardRenderOptions, QueueItem, QueueState},
    css::{self, CssValues, DynamicInput, DynamicState},
    encode::{AvifEncoder, AvifOptions, EncodeSpeed, FaviconEncodeRequest, NativeAvifEncoder},
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("supply benchmark directory")?,
    );
    let raw = std::fs::read(dir.join("input.rgb"))?;
    let frames: Vec<_> = raw
        .as_chunks::<{ 640 * 360 * 3 }>()
        .0
        .iter()
        .map(|p| Frame::new(640, 360, p.to_vec()).map(Arc::new))
        .collect::<Result<_, _>>()?;
    let content = CardContent {
        title: "A track with a reasonably long title".into(),
        subtitle: "Artist name · 1:23 / 4:56".into(),
        playing: true,
        upcoming: (0..4)
            .map(|i| QueueItem {
                title: format!("Next track number {}", i + 1),
                state: if i == 0 {
                    QueueState::Loading
                } else {
                    QueueState::Normal
                },
            })
            .collect(),
    };
    let card = card::render(
        &content,
        &CardRenderOptions {
            fonts_dir: PathBuf::from("C:/Windows/Fonts"),
        },
    )?;
    println!(
        "card {} base64 characters, {} CSS pixels high",
        card.image.len(),
        card.height
    );
    let fallback = card::render(
        &content,
        &CardRenderOptions {
            fonts_dir: dir.join("missing-fonts"),
        },
    )?;
    println!("fallback card {} base64 characters", fallback.image.len());
    let mut state = DynamicState::default();
    let mut times = Vec::new();
    let mut sizes = Vec::new();
    for run in 0..20 {
        let start = Instant::now();
        let mut values = Vec::new();
        let mut glows = Vec::new();
        for (i, frame) in frames.iter().enumerate() {
            glows.push(Arc::new(dt_render::glow_frame(
                frame,
                AudioFeatureFrame {
                    bands: [0.4; 16],
                    bass: 0.4,
                    kick: 0.2,
                    ring: (i + run) as f32 * 60.0,
                },
            )?));
            let mut v = CssValues::from([("bs".into(), "0.4".into()), ("kk".into(), "0.2".into())]);
            if i % 2 == 0 {
                for b in 0..16 {
                    v.insert(format!("b{b}"), "0.4".into());
                }
            }
            if i % 6 == 0 {
                for (key, color) in ["c0", "c1", "c2", "c3", "aa"]
                    .into_iter()
                    .zip(dt_render::edge_colors(frame)?)
                {
                    v.insert(key.into(), color);
                }
                v.insert("pr".into(), "0.3".into());
            }
            values.push(v);
        }
        let built = css::build_dynamic(
            &NativeAvifEncoder,
            &mut state,
            DynamicInput {
                frames: &frames,
                glows: &glows,
                values: &values,
                card: &card,
                playing: true,
                chaos: 11.0,
                prepared: None,
                cheap: false,
            },
        )?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        sizes.push(built.css.chars().count());
    }
    println!(
        "writes ms {times:?}; css chars {sizes:?}; quality {}",
        state.quality
    );
    // Exact-rate, single-play clip used by the external ffprobe/Chromium checks.
    let avif = NativeAvifEncoder.encode(
        &frames,
        &AvifOptions {
            quality: 60,
            speed: EncodeSpeed::Favicon,
            durations_ms: dt_render::encode::frame_durations(frames.len(), 24)?,
            plays: 1,
            threads: 6,
        },
    )?;
    std::fs::write(dir.join("native-exact.avif"), avif)?;
    if dir.join("source.mp4").exists() {
        let request = FaviconEncodeRequest {
            video: dir.join("source.mp4"),
            start: Duration::ZERO,
            duration: Duration::from_secs(1),
            output: dir.join("favicon.svg"),
        };
        let r = dt_render::encode::encode_clip_svg_with(
            &NativeAvifEncoder,
            std::path::Path::new("ffmpeg"),
            &request,
            true,
        )?;
        println!(
            "favicon {} frames {} bytes",
            r.frames,
            std::fs::metadata(r.path)?.len()
        );
    }
    Ok(())
}
