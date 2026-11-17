//! yt-dlp expansion and ID-keyed cache loading, porting `glowtube.expand` and `load`.

use crate::{
    PcmAudio, analyse, decode_audio,
    process::{OwnedChild, check, command},
};
use dt_core::{AudioFeatures, Error, Result, TrackInfo};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

/// Executables and cache location supplied by orchestration.
#[derive(Clone, Debug)]
pub struct DownloadConfig {
    /// yt-dlp executable; every invocation uses `--js-runtimes node` without a shell.
    pub yt_dlp: PathBuf,
    /// ffmpeg executable for stereo PCM decoding and yt-dlp's merge step.
    pub ffmpeg: PathBuf,
    /// Python-compatible cache directory: `<video id>.mp4`, retained across PCM eviction.
    pub cache_dir: PathBuf,
}

/// One local file, single video, or nonempty playlist entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedTrack {
    /// URL or local path to hand to `load`.
    pub url: String,
    /// Playlist/track title, possibly empty until `load` resolves it.
    pub title: String,
}

/// Publish only after metadata, decode and analysis all succeed.
#[derive(Clone, Debug)]
pub struct LoadedTrack {
    /// Resolved metadata and complete 24 fps frame count.
    pub info: TrackInfo,
    /// Decoded stereo audio shared without copying.
    pub pcm: Arc<PcmAudio>,
    /// Features with exactly `info.frames` entries.
    pub features: Arc<AudioFeatures>,
}

fn text<'a>(info: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|&key| info.get(key)?.as_str().filter(|s| !s.is_empty()))
}

fn metadata(url: &str, option: &str, config: &DownloadConfig) -> Result<Value> {
    let mut child = OwnedChild(
        command(&config.yt_dlp)
            .args(["--js-runtimes", "node", "-J", option, "--", url])
            .stdout(Stdio::piped())
            .spawn()?,
    );
    let mut bytes = Vec::new();
    child
        .0
        .stdout
        .take()
        .ok_or(Error::Protocol("yt-dlp stdout"))?
        .read_to_end(&mut bytes)?;
    check(child.0.wait()?, "yt-dlp")?;
    serde_json::from_slice(&bytes).map_err(|_| Error::Protocol("yt-dlp metadata JSON"))
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Expand local files and flat playlists in order, skipping null/empty entries.
/// Single-video results keep the original URL. This function blocks on yt-dlp.
pub fn expand(url: &str, config: &DownloadConfig) -> Result<Vec<ExpandedTrack>> {
    if Path::new(url).is_file() {
        return Ok(vec![ExpandedTrack {
            url: url.into(),
            title: stem(Path::new(url)),
        }]);
    }
    let info = metadata(url, "--flat-playlist", config)?;
    if info.get("_type").and_then(Value::as_str) == Some("playlist") {
        let entries = info.get("entries").and_then(Value::as_array);
        return entries
            .into_iter()
            .flatten()
            .filter(|e| !e.is_null() && e.as_object().is_none_or(|e| !e.is_empty()))
            .map(|entry| {
                let url = if let Some(url) = text(entry, &["url"]) {
                    url.to_owned()
                } else {
                    format!(
                        "https://www.youtube.com/watch?v={}",
                        text(entry, &["id"]).ok_or(Error::Protocol("playlist entry ID"))?
                    )
                };
                Ok(ExpandedTrack {
                    url,
                    title: text(entry, &["title"]).unwrap_or("").into(),
                })
            })
            .collect();
    }
    Ok(vec![ExpandedTrack {
        url: url.into(),
        title: text(&info, &["track", "title"]).unwrap_or("").into(),
    }])
}

fn cache_id(id: &str) -> Result<()> {
    // Metadata must never turn the cache filename/output template into another path.
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.ends_with('.')
        || id
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*%".contains(c))
    {
        return Err(Error::Protocol("unsafe yt-dlp video ID"));
    }
    Ok(())
}

/// Reuse a resolved file or the Python-compatible video-ID cache, otherwise download <=360p.
/// Decode and analyse completely before returning ready PCM; queue ownership stays in the caller.
pub fn load(track: &TrackInfo, config: &DownloadConfig) -> Result<LoadedTrack> {
    let mut info = track.clone();
    let path = if let Some(path) = track.path.as_ref().filter(|p| p.is_file()) {
        path.clone()
    } else if Path::new(&track.url).is_file() {
        let path = Path::new(&track.url);
        info.id = stem(path);
        info.title.clone_from(&info.id);
        info.artist = "local file".into();
        path.canonicalize()?
    } else {
        let meta = metadata(&track.url, "--no-playlist", config)?;
        let id = text(&meta, &["id"]).ok_or(Error::Protocol("yt-dlp video ID"))?;
        cache_id(id)?;
        info.id = id.into();
        info.title = text(&meta, &["track", "title"]).unwrap_or(id).into();
        info.artist = text(&meta, &["artist", "uploader", "channel"])
            .unwrap_or("")
            .into();
        std::fs::create_dir_all(&config.cache_dir)?;
        let path = config.cache_dir.join(format!("{id}.mp4"));
        if !path.is_file() {
            // Remux also handles yt-dlp's single-file fallback, which may be WebM.
            let mut download = command(&config.yt_dlp);
            download.args([
                "--js-runtimes",
                "node",
                "--no-playlist",
                "-q",
                "--merge-output-format",
                "mp4",
                "--remux-video",
                "mp4",
            ]);
            // yt-dlp rejects a bare executable name as a nonexistent ffmpeg-location.
            // Leave PATH lookup to yt-dlp in that case, just as Python does.
            if config.ffmpeg.is_file() {
                download.arg("--ffmpeg-location").arg(&config.ffmpeg);
            }
            let mut child = OwnedChild(
                download
                    .args(["-f", "bv*[height<=360]+ba/b[height<=360]/b", "-o"])
                    .arg(config.cache_dir.join("%(id)s.%(ext)s"))
                    .args(["--", &track.url])
                    .stdout(Stdio::null())
                    .spawn()?,
            );
            check(child.0.wait()?, "yt-dlp")?;
        }
        if !path.is_file() {
            return Err(Error::Protocol("yt-dlp did not produce cached MP4"));
        }
        path
    };
    let pcm = decode_audio(&path, &config.ffmpeg)?;
    let features = analyse(&pcm)?;
    info.frames = features.len() as u64;
    info.path = Some(path);
    Ok(LoadedTrack {
        info,
        pcm: Arc::new(pcm),
        features: Arc::new(features),
    })
}
