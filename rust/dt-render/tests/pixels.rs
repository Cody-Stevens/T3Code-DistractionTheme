//! Pixel checks against Python-generated samples, plus lossless marker round trips.
use base64::Engine;
use dt_core::{AudioFeatureFrame, Frame, constants::CARD_MARKS};
use dt_render::{card::*, edge_colors, glow_frame, light};
#[test]
fn light_and_edges_match_python() {
    assert_eq!(light([0.0; 3]), "#141414");
    assert_eq!(light([255.0, 0.0, 0.0]), "#ff0000");
    let f = Frame::new(
        64,
        36,
        (0..36)
            .flat_map(|y| {
                (0..64)
                    .flat_map(move |x| (0..3).map(move |c| ((x * 7 + y * 13 + c * 67) % 256) as u8))
            })
            .collect(),
    )
    .unwrap();
    assert_eq!(
        edge_colors(&f).unwrap(),
        ["#9ea99b", "#a097a1", "#a093a2", "#9fa69d", "#a1a09f"]
    );
    assert!(edge_colors(&Frame::new(1, 1, vec![0; 3]).unwrap()).is_err());
    let positions = [
        (0, 0),
        (20, 10),
        (40, 35),
        (63, 35),
        (90, 40),
        (100, 60),
        (127, 71),
    ];
    for (bass, kick, ring, expected) in [
        (
            0.0,
            0.0,
            0.0,
            [
                [0, 0, 0],
                [0, 0, 0],
                [80, 42, 56],
                [254, 193, 120],
                [1, 1, 1],
                [0, 0, 0],
                [0, 0, 0],
            ],
        ),
        (
            0.7,
            0.8,
            280.0,
            [
                [0, 0, 0],
                [64, 193, 253],
                [213, 235, 255],
                [255, 235, 213],
                [229, 255, 232],
                [108, 141, 255],
                [0, 0, 0],
            ],
        ),
        (
            1.0,
            1.0,
            750.0,
            [
                [4, 41, 78],
                [158, 255, 235],
                [234, 255, 255],
                [255, 238, 224],
                [231, 255, 252],
                [210, 170, 255],
                [17, 42, 55],
            ],
        ),
    ] {
        let glow = glow_frame(
            &f,
            AudioFeatureFrame {
                bands: [0.0; 16],
                bass,
                kick,
                ring,
            },
        )
        .unwrap();
        assert_eq!((glow.width(), glow.height()), (128, 72));
        for ((x, y), color) in positions.into_iter().zip(expected) {
            let i = (y * 128 + x) * 3;
            for (c, v) in color.into_iter().enumerate() {
                assert!(
                    (i32::from(glow.pixels()[i + c]) - v).abs() <= 12,
                    "glow at {x},{y}, channel {c}: {} vs {v}",
                    glow.pixels()[i + c]
                );
            }
        }
    }
}
#[test]
fn all_marker_pixels_survive_quantization_and_webp() {
    let mut pixels = Vec::new();
    for y in 0..32 {
        for x in 0..112 {
            pixels.extend(if y < 16 {
                CARD_MARKS[x / 16]
            } else {
                [x as u8, y as u8, 180]
            });
        }
    }
    let frame = Frame::new(112, 32, pixels).unwrap();
    let b64 = encode(&frame).unwrap();
    assert!(b64.len() <= 9000);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).unwrap();
    let mut decoded = vec![0; decoder.output_buffer_size().unwrap()];
    decoder.read_image(&mut decoded).unwrap();
    for (old, new) in frame
        .pixels()
        .as_chunks::<3>()
        .0
        .iter()
        .zip(decoded.as_chunks::<3>().0.iter())
    {
        if CARD_MARKS.iter().any(|m| old == m) {
            assert_eq!(old, new);
        }
    }
}
#[test]
fn cards_keep_reference_geometry_with_missing_fonts() {
    for n in 0..=5 {
        let content = CardContent {
            title: "A title".into(),
            subtitle: "A subtitle".into(),
            playing: n % 2 == 0,
            upcoming: (0..n)
                .map(|i| QueueItem {
                    title: format!("Track {i}"),
                    state: QueueState::Normal,
                })
                .collect(),
        };
        let card = render(
            &content,
            &CardRenderOptions {
                fonts_dir: std::path::PathBuf::from("missing-test-fonts"),
            },
        )
        .unwrap();
        assert_eq!(card.height, 204 + 27 * n.clamp(1, 4));
        assert!(card.image.len() <= 9000);
        if n == 4 {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&card.image)
                .unwrap();
            let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).unwrap();
            let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
            decoder.read_image(&mut pixels).unwrap();
            let colors = std::collections::BTreeSet::<_>::from_iter(
                pixels.as_chunks::<3>().0.iter().copied(),
            );
            assert!(colors.len() <= 16);
            for mark in CARD_MARKS {
                assert!(colors.contains(&mark));
            }
        }
    }
}
