//! Container/timing and CSS budget invariants use synthetic frames and isolated encoders.
use dt_core::{Frame, Result};
use dt_render::{
    card::Card,
    css::{self, DynamicInput, DynamicState},
    encode::*,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
fn frames(n: usize) -> Vec<Arc<Frame>> {
    (0..n)
        .map(|i| {
            Arc::new(
                Frame::new(64, 36, [20 + (i * 7 % 200) as u8, 90, 160].repeat(64 * 36)).unwrap(),
            )
        })
        .collect()
}
fn options(n: usize) -> AvifOptions {
    AvifOptions {
        quality: 35,
        speed: EncodeSpeed::Fast,
        durations_ms: frame_durations(n, 24).unwrap(),
        plays: 1,
        threads: 2,
    }
}
fn find_box<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 0;
    while offset + 8 <= data.len() {
        let len = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        if len < 8 || offset + len > data.len() {
            return None;
        }
        let name = &data[offset + 4..offset + 8];
        let payload = &data[offset + 8..offset + len];
        if name == kind {
            return Some(payload);
        }
        if matches!(
            name,
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts"
        ) && let Some(found) = find_box(payload, kind)
        {
            return Some(found);
        }
        offset += len;
    }
    None
}
#[test]
fn cumulative_duration_has_no_drift_or_tie_error() {
    let times = frame_durations(24 * 46, 24).unwrap();
    assert_eq!(&times[..6], &[42, 41, 42, 42, 41, 42]);
    assert_eq!(times.iter().sum::<u32>(), 46000);
    assert_eq!(frame_durations(4, 16).unwrap(), [62, 63, 63, 62]);
    assert!(frame_durations(1, 0).is_err());
    assert!(frame_durations(1, 1001).is_err());
}
#[test]
fn native_sequence_contains_exact_timing_and_one_play() {
    let data = NativeAvifEncoder.encode(&frames(24), &options(24)).unwrap();
    assert!(data.windows(4).any(|p| p == b"avis"));
    let stts = find_box(&data, b"stts").unwrap();
    let runs = u32::from_be_bytes(stts[4..8].try_into().unwrap());
    let mut count = 0;
    let mut duration = 0;
    for i in 0..runs as usize {
        let p = &stts[8 + i * 8..16 + i * 8];
        let n = u32::from_be_bytes(p[..4].try_into().unwrap());
        let d = u32::from_be_bytes(p[4..].try_into().unwrap());
        count += n;
        duration += n * d;
    }
    assert_eq!(count, 24);
    assert_eq!(duration, 1000);
    let elst = find_box(&data, b"elst").unwrap();
    assert_eq!(elst[3] & 1, 0, "one play must not repeat");
    let mut o = options(8);
    o.plays = 0;
    let looping = NativeAvifEncoder.encode(&frames(8), &o).unwrap();
    assert_eq!(find_box(&looping, b"elst").unwrap()[3] & 1, 1);
    assert!(NativeAvifEncoder.encode(&frames(1), &options(1)).is_ok());
}
#[test]
fn invalid_codec_inputs_fail_before_starting_processes() {
    let f = frames(2);
    let mut o = options(2);
    for encoder in [
        &NativeAvifEncoder as &dyn AvifEncoder,
        &FfmpegAvifEncoder {
            ffmpeg: "does-not-exist".into(),
            work_dir: "unused".into(),
        },
        &PersistentAvifEncoder::new("does-not-exist"),
    ] {
        assert!(encoder.encode(&[], &o).is_err());
        o.durations_ms = vec![42];
        assert!(encoder.encode(&f, &o).is_err());
        o = options(2);
        o.quality = 101;
        assert!(encoder.encode(&f, &o).is_err());
        o = options(2);
        let mixed = vec![
            f[0].clone(),
            Arc::new(Frame::new(1, 1, vec![0; 3]).unwrap()),
        ];
        assert!(encoder.encode(&mixed, &o).is_err());
    }
}
struct SizedEncoder {
    bytes: usize,
    calls: Mutex<Vec<(u32, u8, EncodeSpeed)>>,
}
impl AvifEncoder for SizedEncoder {
    fn encode(&self, f: &[Arc<Frame>], o: &AvifOptions) -> Result<Vec<u8>> {
        self.calls
            .lock()
            .unwrap()
            .push((f[0].width(), o.quality, o.speed));
        Ok(vec![42; self.bytes])
    }
}
#[test]
fn every_write_carries_all_layers_and_refuses_overflow() {
    let encoder = SizedEncoder {
        bytes: 300,
        calls: Mutex::new(vec![]),
    };
    let frames = frames(8);
    let glows = frames.clone();
    let values = vec![BTreeMap::from([("bs".into(), "0.3".into())]); 8];
    let card = Card {
        image: "YWJj".into(),
        height: 285,
    };
    let mut state = DynamicState::default();
    let input = || DynamicInput {
        extra_css: "",
        frames: &frames,
        glows: &glows,
        values: &values,
        card: &card,
        playing: true,
        chaos: 11.0,
        prepared: None,
        cheap: false,
    };
    let first = css::build_dynamic(&encoder, &mut state, input()).unwrap();
    let second = css::build_dynamic(&encoder, &mut state, input()).unwrap();
    for css in [&first.css, &second.css] {
        assert!(css.len() <= 49500);
        assert!(!css.contains('\\'));
        for part in [
            "--vid:url(\"data:image/avif;base64,",
            "--glow:url(\"data:image/avif;base64,",
            "--cardimg:url(\"data:image/webp;base64,",
            "--cardh:285px",
        ] {
            assert!(css.contains(part));
        }
    }
    assert!(first.css.contains("@keyframes d1{"));
    assert!(second.css.contains("@keyframes d2{"));
    let prepared = EncodedVideo {
        base64: "YWJj".into(),
        width: 640,
        quality: 45,
    };
    let mut ahead = input();
    ahead.prepared = Some(&prepared);
    let used = css::build_dynamic(&encoder, &mut state, ahead).unwrap();
    assert!(used.css.contains("base64,YWJj\")}"));
    assert!(state.quality.contains("ahead"));
    let huge = SizedEncoder {
        bytes: 50000,
        calls: Mutex::new(vec![]),
    };
    assert!(css::build_dynamic(&huge, &mut state, input()).is_err());
    let mut paused = input();
    paused.playing = false;
    assert!(
        !css::build_dynamic(&encoder, &mut state, paused)
            .unwrap()
            .css
            .contains("@keyframes d5{")
    );
}
#[test]
fn size_search_probes_fast_then_encodes_ahead() {
    let encoder = SizedEncoder {
        bytes: 300,
        calls: Mutex::new(vec![]),
    };
    let result = encode_to_fit(&encoder, &frames(8), 2000, 42)
        .unwrap()
        .unwrap();
    assert_eq!(result.width, 640);
    assert!(result.base64.len() <= 2000);
    let calls = encoder.calls.lock().unwrap();
    assert_eq!(calls[0], (640, 20, EncodeSpeed::Fast));
    assert_eq!(calls[1], (640, 45, EncodeSpeed::Fast));
    assert_eq!(calls[2].2, EncodeSpeed::Ahead);
    assert!(encode_video(&encoder, &frames(8), usize::MAX).is_err());
    assert!(encode_to_fit(&encoder, &[], 1000, 42).is_err());
}

#[test]
fn covered_write_animates_values_and_glow_over_one_still() {
    struct CountEncoder(Mutex<Vec<usize>>);
    impl AvifEncoder for CountEncoder {
        fn encode(&self, frames: &[Arc<Frame>], _: &AvifOptions) -> Result<Vec<u8>> {
            self.0.lock().unwrap().push(frames.len());
            Ok(vec![42; 300])
        }
    }
    let encoder = CountEncoder(Mutex::new(vec![]));
    let video = frames(2);
    let glows = frames(60);
    let values = vec![BTreeMap::from([("bs".into(), "0.3".into())]); 60];
    let card = Card {
        image: "YWJj".into(),
        height: 285,
    };
    let mut state = DynamicState::default();
    let input = || DynamicInput {
        extra_css: "",
        frames: &video[..1],
        glows: &glows,
        values: &values,
        card: &card,
        playing: true,
        chaos: 11.0,
        prepared: None,
        cheap: true,
    };
    let built = css::build_dynamic(&encoder, &mut state, input()).unwrap();
    assert!(built.css.contains("animation:d1 2.500s linear forwards"));
    assert!(built.css.contains("100.00%{--bs:0.3}"));
    assert_eq!(*encoder.0.lock().unwrap(), [60, 1]);
    let mut mismatched = input();
    mismatched.frames = &video;
    assert!(css::build_dynamic(&encoder, &mut state, mismatched).is_err());
    let mut mismatched = input();
    mismatched.glows = &glows[..59];
    assert!(css::build_dynamic(&encoder, &mut state, mismatched).is_err());
}

#[test]
fn diagnostic_switches_reach_builder_and_budget() {
    // Each child gets its own environment: the test suite never mutates global process state.
    let Ok(case) = std::env::var("DT_RENDER_DIAGNOSTIC_TEST") else {
        for (case, measure, loading) in [
            ("off", None, None),
            ("off", Some(""), Some("")),
            ("measure", Some("1"), None),
            ("loading", None, Some("1")),
            ("both", Some("0"), Some("0")),
        ] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", "diagnostic_switches_reach_builder_and_budget"])
                .env("DT_RENDER_DIAGNOSTIC_TEST", case)
                .env_remove("GLOWTUBE_MEASURE")
                .env_remove("GLOWTUBE_SHOW_LOADING");
            if let Some(value) = measure {
                child.env("GLOWTUBE_MEASURE", value);
            }
            if let Some(value) = loading {
                child.env("GLOWTUBE_SHOW_LOADING", value);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{case}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    };
    let expected = match case.as_str() {
        "off" => include_str!("golden/diagnostics_off.css"),
        "measure" => include_str!("golden/diagnostics_measure.css"),
        "loading" => include_str!("golden/diagnostics_loading.css"),
        "both" => include_str!("golden/diagnostics_both.css"),
        _ => panic!("unknown diagnostic test case"),
    };
    assert_eq!(css::diagnostic_css_from_env(), expected);
    let video = frames(1);
    let values = vec![BTreeMap::from([("bs".into(), "0.3".into())])];
    let card = Card {
        image: "YWJj".into(),
        height: 285,
    };
    let encoder = SizedEncoder {
        bytes: 300,
        calls: Mutex::new(vec![]),
    };
    let built = css::build_dynamic(
        &encoder,
        &mut DynamicState::default(),
        DynamicInput {
            extra_css: "",
            frames: &video,
            glows: &[],
            values: &values,
            card: &card,
            playing: false,
            chaos: 11.0,
            prepared: None,
            cheap: false,
        },
    )
    .unwrap();
    assert!(built.css.contains(expected));
    assert_eq!(
        built.css.contains("background:#ff00ff!important"),
        case == "measure" || case == "both"
    );
    assert_eq!(
        built.css.contains("body:has(img.hidden"),
        case == "loading" || case == "both"
    );
    // The known 400-character base64 payload is the only part not reserved by video_room.
    assert_eq!(
        built.css.len() + built.video_room - 400,
        dt_core::constants::CSS_LIMIT
    );
    assert!(!built.css.contains('\\'));
}
