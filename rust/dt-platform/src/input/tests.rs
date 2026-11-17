use super::*;
use std::{
    sync::{atomic::AtomicUsize, mpsc},
    time::Instant,
};

fn strip() -> Frame {
    Frame::new(1200, 17, vec![0; 1200 * 17 * 3]).unwrap()
}
const CENTER: ScreenPoint = ScreenPoint { x: 600, y: 8 };
fn put(frame: &mut Frame, x: usize, y: usize, rgb: [u8; 3]) {
    let offset = (y * frame.width() as usize + x) * 3;
    frame.pixels_mut()[offset..offset + 3].copy_from_slice(&rgb);
}

#[test]
fn python_synthetic_cards_hit_all_28_targets_at_100_and_150_percent() {
    let mut hits = 0;
    for fixture in [
        include_str!("../../tests/fixtures/cards-100.rle"),
        include_str!("../../tests/fixtures/cards-150.rle"),
    ] {
        for line in fixture.lines().filter(|line| !line.starts_with('#')) {
            let mut words = line.split_whitespace();
            let name = words.next().unwrap();
            let target = match name {
                "toggle" => CardTarget::Toggle,
                "scrub" => CardTarget::Scrub,
                "row0" => CardTarget::QueueRow(0),
                "row2" => CardTarget::QueueRow(2),
                "linkplay" => CardTarget::LinkPlay,
                "linkqueue" => CardTarget::LinkQueue,
                _ => panic!("unknown fixture target"),
            };
            let mut rgb = Vec::new();
            for word in words {
                let (count, hex) = word.split_once(':').unwrap();
                let pixel = u32::from_str_radix(hex, 16).unwrap();
                for _ in 0..count.parse::<usize>().unwrap() {
                    rgb.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8]);
                }
            }
            let pixels = Frame::new(1200, 17, rgb).unwrap();
            let hit = hit_test(&pixels, CENTER, ClickConfig::default())
                .unwrap()
                .unwrap();
            assert_eq!(hit.target, target, "{name}");
            if target == CardTarget::Scrub {
                assert!((0.0..=1.0).contains(&hit.fraction));
            }
            hits += 1;
        }
    }
    assert_eq!(hits, 28);
    println!("synthetic card targets: {hits}/28");
}

#[test]
fn random_video_like_noise_has_at_most_one_false_hit_in_400_clicks() {
    let mut random = 0x53c4_ba91_u32;
    let mut next = || {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        random
    };
    let mut false_hits = 0;
    for _ in 0..400 {
        let mut pixels = strip();
        // Independent chroma, horizontal correlation, and sparse exact marker speckles.
        for y in 0..17 {
            for block in 0..300 {
                let n = next();
                let rgb = if n % 31 == 0 {
                    CARD_MARKS[(next() as usize) % 7]
                } else {
                    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
                };
                for x in block * 4..block * 4 + 4 {
                    put(&mut pixels, x, y, rgb);
                }
            }
        }
        if hit_test(&pixels, CENTER, ClickConfig::default())
            .unwrap()
            .is_some()
        {
            false_hits += 1;
        }
    }
    assert!(false_hits <= 1, "{false_hits}/400 false hits");
    println!("noise false hits: {false_hits}/400");
}

#[test]
fn exact_band_threshold_tolerance_and_marker_order_match_python() {
    let mut frame = strip();
    for count in 0..158 {
        let x = 586 + count % 29;
        let y = count / 29;
        put(&mut frame, x, y, CARD_MARKS[0].map(|channel| channel + 10));
        let hit = hit_test(&frame, CENTER, ClickConfig::default()).unwrap();
        assert_eq!(hit.is_some(), count == 157);
    }
    assert!(
        hit_test(
            &frame,
            CENTER,
            ClickConfig {
                minimum_fill: 0.33,
                ..Default::default()
            }
        )
        .unwrap()
        .is_none()
    );
    let mut outside = strip();
    for y in 0..17 {
        for x in 586..615 {
            put(
                &mut outside,
                x,
                y,
                CARD_MARKS[0].map(|channel| channel + 11),
            );
        }
    }
    assert!(
        hit_test(&outside, CENTER, ClickConfig::default())
            .unwrap()
            .is_none()
    );
    for &fill in &[f32::NAN, f32::INFINITY, -0.01, 1.01] {
        assert!(
            hit_test(
                &frame,
                CENTER,
                ClickConfig {
                    minimum_fill: fill,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    assert!(
        hit_test(&frame, ScreenPoint { x: -1, y: 8 }, ClickConfig::default())
            .unwrap()
            .is_none()
    );
    assert!(
        hit_test(
            &frame,
            ScreenPoint { x: i32::MAX, y: 8 },
            ClickConfig::default()
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn scrub_requires_more_than_60_and_prefers_central_row() {
    let mut frame = strip();
    for x in 570..=630 {
        put(&mut frame, x, 8, MARK_TRACK);
    }
    assert!(
        hit_test(&frame, CENTER, ClickConfig::default())
            .unwrap()
            .is_none()
    );
    put(&mut frame, 631, 8, MARK_FILL);
    let hit = hit_test(&frame, CENTER, ClickConfig::default())
        .unwrap()
        .unwrap();
    assert_eq!(hit.target, CardTarget::Scrub);
    assert!((hit.fraction - 30.0 / 61.0).abs() < 1e-6);
    for x in 500..=700 {
        put(&mut frame, x, 7, MARK_TRACK);
    }
    assert_eq!(
        hit_test(&frame, CENTER, ClickConfig::default())
            .unwrap()
            .unwrap(),
        hit
    );
    for x in 570..=631 {
        put(&mut frame, x, 8, [0; 3]);
    }
    assert_eq!(
        hit_test(&frame, CENTER, ClickConfig::default())
            .unwrap()
            .unwrap()
            .fraction,
        0.5
    );
}

#[test]
fn panel_threshold_uses_physical_dpi_including_negative_monitors() {
    for (right, dpi, boundary) in [
        (2000, 96, 1660),
        (2000, 144, 1490),
        (-100, 144, -610),
        (2000, 0, 1660),
    ] {
        assert!(point_in_right_panel(
            ScreenPoint { x: boundary, y: 0 },
            right,
            dpi
        ));
        assert!(!point_in_right_panel(
            ScreenPoint {
                x: boundary - 1,
                y: 0
            },
            right,
            dpi
        ));
    }
}

#[test]
fn all_markers_and_equal_count_ties_use_reference_order() {
    for (marker, target) in CARD_MARKS.into_iter().zip(TARGETS) {
        let mut frame = strip();
        for y in 0..17 {
            for x in 586..615 {
                put(&mut frame, x, y, marker);
            }
        }
        assert_eq!(
            hit_test(&frame, CENTER, ClickConfig::default())
                .unwrap()
                .unwrap()
                .target,
            target
        );
    }
    let mut frame = strip();
    for i in 0..316 {
        put(
            &mut frame,
            586 + i % 29,
            i / 29,
            CARD_MARKS[usize::from(i >= 158)],
        );
    }
    assert_eq!(
        hit_test(&frame, CENTER, ClickConfig::default())
            .unwrap()
            .unwrap()
            .target,
        CardTarget::Toggle
    );
}

#[test]
fn watchers_stop_join_and_preserve_external_stop() {
    let stop = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();
    let mut watcher = spawn_watcher("test-input", stop.clone(), move || {
        let _ = sender.send(());
        true
    })
    .unwrap();
    receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    let start = Instant::now();
    watcher.stop().unwrap();
    watcher.stop().unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(!stop.load(Ordering::Acquire));
    let calls = Arc::new(AtomicUsize::new(0));
    let on_thread = calls.clone();
    let external = Arc::new(AtomicBool::new(true));
    drop(
        spawn_watcher("already-stopped", external, move || {
            on_thread.fetch_add(1, Ordering::Relaxed);
            true
        })
        .unwrap(),
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[cfg(windows)]
#[test]
#[ignore = "manual one-shot read-only desktop strip; never clicks or saves pixels"]
fn live_gdi_strip_once() {
    enable_dpi_awareness().unwrap();
    let foreground = foreground_is_t3().unwrap();
    let point = crate::win32::cursor_position().unwrap();
    let pixels = crate::win32::capture_strip(point).unwrap();
    assert_eq!(
        (pixels.width(), pixels.height(), pixels.pixels().len()),
        (1200, 17, 61200)
    );
    println!("foreground is T3: {foreground}; GDI read: 1200x17 RGB24");
}
