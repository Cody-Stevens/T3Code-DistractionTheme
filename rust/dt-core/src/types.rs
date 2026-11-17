use std::{path::PathBuf, time::Duration};

use crate::{Error, Result, constants::BANDS};

/// Owned, tightly packed RGB24 frame in row-major order, with no padding or alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Frame {
    /// Validate nonzero dimensions and exactly `width * height * 3` bytes.
    /// Dimensions are not restricted to decode size: glow and card frames use this type too.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self> {
        let expected = usize::try_from(width)
            .ok()
            .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
            .and_then(|n| n.checked_mul(3))
            .ok_or(Error::InvalidInput("RGB24 dimensions overflow"))?;
        if width == 0 || height == 0 || pixels.len() != expected {
            return Err(Error::InvalidInput(
                "RGB24 dimensions and buffer length differ",
            ));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Width in physical pixels.
    pub fn width(&self) -> u32 {
        self.width
    }
    /// Height in physical pixels.
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Borrow packed RGB bytes.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    /// Change colors without changing the validated buffer length.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }
    /// Consume the frame and recover the packed buffer without copying.
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

/// Metadata portion of `glowtube.Track`; PCM, load state and analysis belong to the player/media.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackInfo {
    /// Original URL or local file name.
    pub url: String,
    /// yt-dlp ID, or local file stem; empty until resolved.
    pub id: String,
    /// Human-readable track name.
    pub title: String,
    /// Artist, uploader, channel, or `local file`.
    pub artist: String,
    /// Resolved downloaded or local source; absent until loaded.
    pub path: Option<PathBuf>,
    /// Number of complete 24 fps frames, truncated from PCM duration.
    pub frames: u64,
}

impl TrackInfo {
    /// An unresolved queue entry, matching Python's `Track(url)` defaults.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            id: String::new(),
            title: "Loading…".into(),
            artist: String::new(),
            path: None,
            frames: 0,
        }
    }
}

/// Values for one 24 fps frame from `glowtube.analyse`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioFeatureFrame {
    /// Normalized log-frequency band envelopes, low to high, each in 0..=1.
    pub bands: [f32; BANDS],
    /// Bass envelope in 0..=1.
    pub bass: f32,
    /// Kick envelope in 0..=1.
    pub kick: f32,
    /// Nonnegative shockwave radius in screen pixels, advancing at 1440 px/s.
    pub ring: f32,
}

/// Equal-length per-frame arrays; construction validates finite values and normalized envelopes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioFeatures {
    bands: Vec<[f32; BANDS]>,
    bass: Vec<f32>,
    kick: Vec<f32>,
    ring: Vec<f32>,
}

impl AudioFeatures {
    /// Construct analysis arrays; empty input is valid for zero complete video frames.
    pub fn new(
        bands: Vec<[f32; BANDS]>,
        bass: Vec<f32>,
        kick: Vec<f32>,
        ring: Vec<f32>,
    ) -> Result<Self> {
        if [bass.len(), kick.len(), ring.len()]
            .iter()
            .any(|&n| n != bands.len())
        {
            return Err(Error::InvalidInput("audio feature array lengths differ"));
        }
        if bands
            .iter()
            .flatten()
            .chain(&bass)
            .chain(&kick)
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
            || ring.iter().any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err(Error::InvalidInput("invalid audio feature value"));
        }
        Ok(Self {
            bands,
            bass,
            kick,
            ring,
        })
    }

    /// Number of complete video frames.
    pub fn len(&self) -> usize {
        self.bands.len()
    }
    /// Whether there are no complete video frames.
    pub fn is_empty(&self) -> bool {
        self.bands.is_empty()
    }
    /// Per-frame spectrum bands.
    pub fn bands(&self) -> &[[f32; BANDS]] {
        &self.bands
    }
    /// Per-frame bass envelopes.
    pub fn bass(&self) -> &[f32] {
        &self.bass
    }
    /// Per-frame kick envelopes.
    pub fn kick(&self) -> &[f32] {
        &self.kick
    }
    /// Per-frame shockwave radii, in screen pixels.
    pub fn ring(&self) -> &[f32] {
        &self.ring
    }
    /// Copy one frame's features; return `None` past the end.
    pub fn get(&self, index: usize) -> Option<AudioFeatureFrame> {
        Some(AudioFeatureFrame {
            bands: *self.bands.get(index)?,
            bass: self.bass[index],
            kick: self.kick[index],
            ring: self.ring[index],
        })
    }
}

/// Sample-accurate position of audio reaching the speakers (`Player.media_time`).
/// Account for device latency, clamp before zero, freeze on pause, and jump on seek/track change.
/// This is media time, not a monotonic wall clock. Implementations must permit concurrent reads.
pub trait Clock: Send + Sync {
    /// Current audible media position; do not substitute queued sample count or wall uptime.
    fn media_time(&self) -> Duration;
}

/// Card target shared by the renderer and platform pixel reader, without a sibling dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardTarget {
    /// Play/pause disc.
    Toggle,
    /// Play the copied YouTube link immediately.
    LinkPlay,
    /// Append the copied YouTube link.
    LinkQueue,
    /// Zero-based visible queue row (0..4); the player interprets reorder behavior.
    QueueRow(u8),
    /// Progress bar; the hit carries a separate fraction.
    Scrub,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::*;

    #[test]
    fn rgb24_is_tightly_packed_and_mutable() {
        let mut frame = Frame::new(2, 1, vec![0; 6]).unwrap();
        frame.pixels_mut()[3..].copy_from_slice(&[11, 22, 33]);
        assert_eq!((frame.width(), frame.height()), (2, 1));
        assert_eq!(frame.pixels(), &[0, 0, 0, 11, 22, 33]);
        assert_eq!(frame.clone().into_pixels(), frame.pixels());
    }

    #[test]
    fn malformed_frames_are_rejected_without_allocating() {
        for (w, h, bytes) in [
            (0, 1, vec![]),
            (1, 1, vec![0; 4]),
            (u32::MAX, u32::MAX, vec![]),
        ] {
            assert!(Frame::new(w, h, bytes).is_err());
        }
    }

    #[test]
    fn features_preserve_aligned_frame_values() {
        let f = AudioFeatures::new(
            vec![[0.2; BANDS]; 2],
            vec![0.1, 0.9],
            vec![0.0, 1.0],
            vec![60.0, 0.0],
        )
        .unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f.get(1).unwrap().kick, 1.0);
        assert_eq!(f.get(1).unwrap().ring, 0.0);
        assert!(f.get(2).is_none());
        assert!(AudioFeatures::default().is_empty());
        assert!(AudioFeatures::new(vec![], vec![0.0], vec![], vec![]).is_err());
        for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(
                AudioFeatures::new(vec![[value; BANDS]], vec![0.0], vec![0.0], vec![0.0]).is_err()
            );
        }
        assert!(AudioFeatures::new(vec![[0.0; BANDS]], vec![0.0], vec![0.0], vec![-1.0]).is_err());
    }

    #[test]
    fn clock_can_be_shared_as_a_trait_object() {
        struct Fixed;
        impl Clock for Fixed {
            fn media_time(&self) -> Duration {
                Duration::from_secs(3)
            }
        }
        let clock: std::sync::Arc<dyn Clock> = std::sync::Arc::new(Fixed);
        assert_eq!(clock.media_time(), Duration::from_secs(3));
    }

    #[test]
    fn timing_and_payload_budgets_match_python() {
        assert_eq!(SAMPLES_PER_FRAME, 2000);
        assert_eq!(
            WRITE_INTERVAL.as_millis() * u128::from(FPS),
            u128::from(WRITE_STEP_FRAMES) * 1000
        );
        assert_eq!(WRITE_SPAN_FRAMES, WRITE_STEP_FRAMES + 2);
        const { assert!(CSS_LIMIT < T3_CSS_LIMIT) };
        const { assert!(FAVICON_MIN_BYTES > FAVICON_LOADER_LIMIT) };
        let track = TrackInfo::new("example.mp4");
        assert_eq!(track.title, "Loading…");
        assert!(track.path.is_none());
    }

    #[test]
    fn io_error_keeps_its_source() {
        use std::error::Error as _;
        let error = Error::from(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(error.source().is_some());
        assert_eq!(Error::Http { status: 404 }.to_string(), "HTTP 404");
    }
}
