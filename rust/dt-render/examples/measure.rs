//! Offline encoder/card benchmark; writes synthetic artifacts only to the supplied directory.
use dt_core::Frame;
use dt_render::{
    card::{self, CardContent, CardRenderOptions, QueueItem, QueueState},
    encode::{AvifEncoder, AvifOptions, EncodeSpeed, FfmpegAvifEncoder},
};
use std::{path::PathBuf, sync::Arc, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("supply output directory")?,
    );
    std::fs::create_dir_all(&dir)?;
    let raw = std::fs::read(dir.join("input.rgb"))?;
    let frames: Vec<_> = raw
        .as_chunks::<{ 640 * 360 * 3 }>()
        .0
        .iter()
        .map(|p| Frame::new(640, 360, p.to_vec()).map(Arc::new))
        .collect::<Result<_, _>>()?;
    let backend = std::env::args().nth(2).unwrap_or_else(|| "native".into());
    let encoder: Box<dyn AvifEncoder> = match backend.as_str() {
        "native" => Box::new(dt_render::encode::NativeAvifEncoder),
        "persistent" => Box::new(dt_render::encode::PersistentAvifEncoder::new("ffmpeg")),
        "oneshot" => Box::new(FfmpegAvifEncoder {
            ffmpeg: "ffmpeg".into(),
            work_dir: dir.clone(),
        }),
        _ => return Err("backend must be native, persistent, or oneshot".into()),
    };
    for (label, speed) in [("fast", EncodeSpeed::Fast), ("ahead", EncodeSpeed::Ahead)] {
        for quality in [20, 35, 50] {
            let options = AvifOptions {
                quality,
                speed,
                durations_ms: vec![42; frames.len()],
                plays: 1,
                threads: 4,
            };
            let mut times = Vec::new();
            let mut size = 0;
            for i in 0..6 {
                let t = Instant::now();
                let data = encoder.encode(&frames, &options)?;
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                size = data.len();
                std::fs::write(
                    dir.join(format!("rust-{backend}-{label}-q{quality}-{i}.avif")),
                    data,
                )?;
            }
            println!("{label} q{quality} {size} bytes, ms {times:?}");
        }
    }
    let options = AvifOptions {
        quality: 35,
        speed: EncodeSpeed::Fast,
        durations_ms: vec![42; frames.len()],
        plays: 1,
        threads: 4,
    };
    let one = FfmpegAvifEncoder {
        ffmpeg: "ffmpeg".into(),
        work_dir: dir.clone(),
    };
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        one.encode(&frames, &options)?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    println!("one-shot {times:?}");
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
    for fonts_dir in [PathBuf::from("C:/Windows/Fonts"), dir.join("missing-fonts")] {
        let t = Instant::now();
        let card = card::render(
            &content,
            &CardRenderOptions {
                fonts_dir: fonts_dir.clone(),
            },
        )?;
        use base64::Engine;
        std::fs::write(
            dir.join(if fonts_dir.exists() {
                "card.webp"
            } else {
                "fallback.webp"
            }),
            base64::engine::general_purpose::STANDARD.decode(&card.image)?,
        )?;
        println!(
            "card {} chars {}px {:?}",
            card.image.len(),
            card.height,
            t.elapsed()
        );
    }
    Ok(())
}
