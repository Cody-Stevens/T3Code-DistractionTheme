//! Isolated native-process checks. No device, T3 state, token or control port is opened.
#![cfg(windows)]
use distraction::{encoder_process::EncoderProcess, favicon::EncodeEpoch};
use std::{path::PathBuf, time::Duration};

#[test]
fn encoder_process_warms_stops_and_restarts() {
    let encoder = EncoderProcess::new(
        PathBuf::from(env!("CARGO_BIN_EXE_distraction")),
        "unused-ffmpeg".into(),
    );
    let epoch = EncodeEpoch::default();
    encoder.warm(&epoch).unwrap();
    encoder.cancel().unwrap();
    encoder.warm(&epoch).unwrap();
    encoder.cancel().unwrap();
    encoder.cancel().unwrap();
}

#[test]
#[ignore = "requires ffmpeg/ffprobe on PATH; generates temporary synthetic media"]
fn synthetic_media_loads_and_the_real_worker_encodes_a_padded_favicon() {
    use dt_t3::favicon::{ClipJob, Deck};
    use std::process::{Command, Stdio};
    let temp = tempfile::tempdir().unwrap();
    let video = temp.path().join("synthetic.mp4");
    let mut generate = Command::new("ffmpeg");
    generate
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "3",
            "-threads",
            "2",
            "-c:v",
            "mpeg4",
            "-q:v",
            "4",
            "-c:a",
            "aac",
        ])
        .arg(&video)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = dt_platform::process::spawn_hidden(&mut generate).unwrap();
    let status = child.wait().unwrap();
    dt_platform::process::kill_process_tree(&mut child).unwrap();
    assert!(status.success());
    let config = dt_media::DownloadConfig {
        yt_dlp: "unused-yt-dlp".into(),
        ffmpeg: "ffmpeg".into(),
        cache_dir: temp.path().join("cache"),
    };
    let expanded = dt_media::expand(video.to_str().unwrap(), &config).unwrap();
    assert_eq!(expanded.len(), 1);
    let track = dt_media::load(
        &dt_core::TrackInfo {
            id: "test".into(),
            url: expanded[0].url.clone(),
            title: expanded[0].title.clone(),
            artist: String::new(),
            path: None,
            frames: 0,
        },
        &config,
    )
    .unwrap();
    assert!(track.info.frames >= 72);
    assert!(!track.pcm.samples.is_empty());
    assert!(track.features.get(60).is_some());
    let encoder = EncoderProcess::new(
        PathBuf::from(env!("CARGO_BIN_EXE_distraction")),
        "ffmpeg".into(),
    );
    let output = temp.path().join("glowtube-clip-a-test-1.svg");
    let ready = encoder
        .encode(
            ClipJob {
                track_id: track.info.id,
                video: track.info.path.unwrap(),
                deck: Deck::A,
                start: Duration::from_millis(500),
                duration: Duration::from_millis(500),
                output: output.clone(),
                speed: 10,
                quality: 55,
                done_by: Duration::from_secs(1),
            },
            &EncodeEpoch::default(),
        )
        .unwrap();
    assert_eq!(ready.frames, 12);
    assert!(
        std::fs::metadata(&output).unwrap().len() >= dt_core::constants::FAVICON_MIN_BYTES as u64
    );
    assert!(
        std::fs::read_to_string(output)
            .unwrap()
            .contains("data:image/avif;base64,")
    );
    encoder.cancel().unwrap();
}
