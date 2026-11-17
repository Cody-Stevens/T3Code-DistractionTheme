//! Native Windows mock executables exercise tool arguments/cache paths without network or codecs.
#![cfg(windows)]

use dt_core::{Error, TrackInfo};
use dt_media::{
    DownloadConfig, VideoReader, VideoReaderConfig, VideoSource, decode_audio, expand, load,
};
use std::{fs, path::Path};
use tempfile::TempDir;

fn mocks(metadata: &str) -> (TempDir, DownloadConfig) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("metadata.json"), metadata).unwrap();
    fs::write(dir.path().join("pcm.bin"), vec![0; 40000]).unwrap();
    fs::write(dir.path().join("download.bin"), b"synthetic mock media").unwrap();
    let yt_dlp = dir.path().join("yt-dlp.cmd");
    fs::write(&yt_dlp, "@echo off\r\necho %*>>\"%~dp0calls.txt\"\r\nif \"%~3\"==\"-J\" (\r\n type \"%~dp0metadata.json\"\r\n exit /b 0\r\n)\r\ncopy /b \"%~dp0download.bin\" \"%~dp0cache\\video-id.mp4\" >nul\r\nexit /b 0\r\n").unwrap();
    let ffmpeg = dir.path().join("ffmpeg.cmd");
    fs::write(
        &ffmpeg,
        "@echo off\r\necho %*>>\"%~dp0decode.txt\"\r\ntype \"%~dp0pcm.bin\"\r\nexit /b 0\r\n",
    )
    .unwrap();
    let config = DownloadConfig {
        yt_dlp,
        ffmpeg,
        cache_dir: dir.path().join("cache"),
    };
    (dir, config)
}

fn read(dir: &TempDir, name: &str) -> String {
    fs::read_to_string(dir.path().join(name)).unwrap()
}

#[test]
fn expansion_preserves_order_skips_null_and_resolves_id() {
    let (dir, config) = mocks(
        r#"{"_type":"playlist","entries":[null,{}, {"id":"one","title":"First"},{"url":"https://example.invalid/two","title":null}]}"#,
    );
    let tracks = expand("https://example.invalid/list", &config).unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].url, "https://www.youtube.com/watch?v=one");
    assert_eq!(tracks[0].title, "First");
    assert_eq!(tracks[1].url, "https://example.invalid/two");
    assert_eq!(tracks[1].title, "");
    let args = read(&dir, "calls.txt");
    assert!(args.contains("--js-runtimes node -J --flat-playlist --"));
    fs::write(
        dir.path().join("metadata.json"),
        r#"{"track":"Song","title":"Video title"}"#,
    )
    .unwrap();
    let single = expand("https://example.invalid/video", &config).unwrap();
    assert_eq!(single[0].url, "https://example.invalid/video");
    assert_eq!(single[0].title, "Song");
    let local = dir.path().join("local file.mp4");
    fs::write(&local, b"mock").unwrap();
    assert_eq!(
        expand(local.to_str().unwrap(), &config).unwrap()[0].title,
        "local file"
    );
    assert_eq!(read(&dir, "calls.txt").lines().count(), 2);
}

#[test]
fn download_cache_reload_and_local_metadata() {
    let (dir, config) = mocks(
        r#"{"id":"video-id","track":"Song","title":"Video title","artist":"Artist","uploader":"Uploader"}"#,
    );
    let track = TrackInfo::new("https://example.invalid/video");
    let loaded = load(&track, &config).unwrap();
    assert_eq!(loaded.info.id, "video-id");
    assert_eq!(loaded.info.title, "Song");
    assert_eq!(loaded.info.artist, "Artist");
    assert_eq!(
        loaded.info.path.as_deref(),
        Some(config.cache_dir.join("video-id.mp4").as_path())
    );
    assert_eq!(loaded.pcm.samples.len(), 10000);
    assert_eq!(loaded.info.frames, 5);
    assert_eq!(loaded.features.len(), 5);
    assert_eq!(track.path, None);
    let calls = read(&dir, "calls.txt");
    assert_eq!(calls.lines().count(), 2);
    assert!(
        calls
            .lines()
            .all(|line| line.contains("--js-runtimes node"))
    );
    assert!(calls.contains("bv*[height<=360]+ba/b[height<=360]/b"));
    assert!(calls.contains("--merge-output-format mp4 --remux-video mp4"));
    assert!(calls.contains("%(id)s.%(ext)s"));
    let decoded = read(&dir, "decode.txt");
    assert!(decoded.contains("-vn -f s16le -ac 2 -ar 48000 -"));
    load(&track, &config).unwrap(); // Metadata but no second download.
    assert_eq!(read(&dir, "calls.txt").lines().count(), 3);
    load(&loaded.info, &config).unwrap(); // Resolved path bypasses yt-dlp entirely.
    assert_eq!(read(&dir, "calls.txt").lines().count(), 3);
    let local = dir.path().join("local track.wav");
    fs::write(&local, b"mock").unwrap();
    let loaded = load(&TrackInfo::new(local.to_string_lossy()), &config).unwrap();
    assert_eq!(loaded.info.id, "local track");
    assert_eq!(loaded.info.title, "local track");
    assert_eq!(loaded.info.artist, "local file");
}

#[test]
fn malformed_metadata_failed_tools_and_partial_pcm_do_not_publish() {
    let (dir, config) = mocks("not JSON");
    assert!(expand("https://example.invalid/video", &config).is_err());
    for id in ["../outside", "x\\y", "%unsafe", "C:outside", ".."] {
        fs::write(
            dir.path().join("metadata.json"),
            serde_json::json!({"id":id}).to_string(),
        )
        .unwrap();
        assert!(load(&TrackInfo::new("https://example.invalid/video"), &config).is_err());
        assert!(!config.cache_dir.exists());
    }
    fs::write(dir.path().join("pcm.bin"), [0; 5]).unwrap();
    assert!(matches!(
        decode_audio(Path::new("fake"), &config.ffmpeg),
        Err(Error::Protocol("partial stereo PCM frame"))
    ));
    fs::write(
        &config.ffmpeg,
        "@echo off\r\ntype \"%~dp0pcm.bin\"\r\nexit /b 9\r\n",
    )
    .unwrap();
    assert!(matches!(
        decode_audio(Path::new("fake"), &config.ffmpeg),
        Err(Error::ExternalTool { code: Some(9), .. })
    ));
    fs::write(
        &config.yt_dlp,
        "@echo off\r\necho private diagnostic 1>&2\r\nexit /b 7\r\n",
    )
    .unwrap();
    let error = expand("https://example.invalid/video", &config).unwrap_err();
    assert!(matches!(error, Error::ExternalTool { code: Some(7), .. }));
    assert!(!error.to_string().contains("private diagnostic"));
}

#[test]
fn video_pipe_failure_is_reported_and_reaped() {
    let (_dir, config) = mocks("{}");
    fs::write(&config.ffmpeg, "@echo off\r\nexit /b 9\r\n").unwrap();
    let mut reader = VideoReader::open(VideoReaderConfig {
        path: "fake".into(),
        ffmpeg: config.ffmpeg,
    })
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match reader.get(0) {
            Err(Error::ExternalTool { code: Some(9), .. }) => break,
            other => assert!(matches!(other, Ok(None))),
        }
        assert!(
            std::time::Instant::now() < deadline,
            "failed video process was not reported"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    reader.stop().unwrap();
}
