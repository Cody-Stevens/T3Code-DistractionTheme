//! Opt-in real-codec seek/loop tests; synthetic media lives only in private scratch directories.
use dt_core::Frame;
use dt_media::{VideoReader, VideoReaderConfig, VideoSource};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn frame(reader: &mut VideoReader, index: u64) -> Arc<Frame> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(frame) = reader.get(index).unwrap() {
            return frame;
        }
        assert!(Instant::now() < deadline, "ffmpeg frame timeout");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires ffmpeg on PATH; generates synthetic video"]
fn real_pipe_seeks_loops_and_reaps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.mkv");
    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x90:r=24:d=7",
            "-c:v",
            "ffv1",
            "-threads",
            "2",
        ])
        .arg(&path)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let config = VideoReaderConfig {
        path,
        ffmpeg: PathBuf::from("ffmpeg"),
    };
    let mut reader = VideoReader::open(config.clone()).unwrap();
    let zero = frame(&mut reader, 0);
    assert_eq!(
        (zero.width(), zero.height(), zero.pixels().len()),
        (640, 360, 640 * 360 * 3)
    );
    // Move the wanted position until frame zero is older than the two-second retained history.
    for i in 1..=125 {
        frame(&mut reader, i);
    }
    assert!(
        reader.get(0).unwrap().is_none(),
        "loop must restart when older than the buffer"
    );
    assert_eq!(frame(&mut reader, 0).pixels(), zero.pixels());
    reader.start_at(72).unwrap();
    let sought = frame(&mut reader, 72);
    assert_ne!(sought.pixels(), zero.pixels());
    let mut comparison = VideoReader::open(config).unwrap();
    comparison.start_at(72).unwrap();
    assert_eq!(sought.pixels(), frame(&mut comparison, 72).pixels());
    // Decode the last frame, then loop from EOF (which must use oldest, not decoded alone).
    reader.start_at(167).unwrap();
    frame(&mut reader, 167);
    assert!(reader.get(0).unwrap().is_none());
    assert_eq!(frame(&mut reader, 0).pixels(), zero.pixels());
    // A far-ahead request restarts even before the pipe supplies its first frame.
    assert!(reader.get(10000).unwrap().is_none());
    assert!(reader.get(0).unwrap().is_none());
    assert_eq!(frame(&mut reader, 0).pixels(), zero.pixels());
    reader.stop().unwrap();
    comparison.stop().unwrap();
    assert!(reader.get(0).unwrap().is_none());
    reader.stop().unwrap();
}
