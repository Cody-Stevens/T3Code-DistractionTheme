//! PCM and FFT envelopes from `glowtube.load` and `analyse`.

use crate::process::{OwnedChild, check, command};
use dt_core::{AudioFeatures, Error, Result, constants::*};
use rustfft::{FftPlanner, num_complex::Complex};
use std::{io::Read, path::Path, process::Stdio};

pub use crate::output::{AudioOutput, AudioOutputConfig, CpalOutput};

/// Stereo sample frames (`[left, right]`) at `dt_core::constants::SAMPLE_RATE`.
/// Duration is `samples.len() / 48000`, not scalar-sample count.
#[derive(Clone, Debug)]
pub struct PcmAudio {
    /// Signed 16-bit samples, ready for the callback.
    pub samples: Vec<[i16; 2]>,
}

/// Decode a file to 48 kHz stereo s16le, rejecting partial frames and failed ffmpeg exits.
/// This blocking operation belongs on a loader worker, never the theme or device callback.
pub fn decode_audio(path: &Path, ffmpeg: &Path) -> Result<PcmAudio> {
    let mut child = OwnedChild(
        command(ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-vn", "-f", "s16le", "-ac", "2", "-ar", "48000", "-"])
            .stdout(Stdio::piped())
            .spawn()?,
    );
    let mut pipe = child
        .0
        .stdout
        .take()
        .ok_or(Error::Protocol("ffmpeg stdout"))?;
    let mut buffer = [0; 65536];
    let mut filled = 0;
    let mut samples = Vec::new();
    loop {
        let n = pipe.read(&mut buffer[filled..])?;
        filled += n;
        let complete = filled / 4 * 4;
        samples.extend(buffer[..complete].as_chunks::<4>().0.iter().map(|s| {
            [
                i16::from_le_bytes([s[0], s[1]]),
                i16::from_le_bytes([s[2], s[3]]),
            ]
        }));
        buffer.copy_within(complete..filled, 0);
        filled -= complete;
        if n == 0 {
            break;
        }
    }
    check(child.0.wait()?, "ffmpeg")?;
    if filled != 0 {
        return Err(Error::Protocol("partial stereo PCM frame"));
    }
    Ok(PcmAudio { samples })
}

fn percentile(sorted: &[f32], percent: f64) -> f64 {
    let index = (sorted.len() - 1) as f64 * percent / 100.0;
    let lo = index.floor() as usize;
    let hi = index.ceil() as usize;
    f64::from(sorted[lo]) + (f64::from(sorted[hi]) - f64::from(sorted[lo])) * index.fract()
}

fn normalise_envelope(values: &mut [f32], low: f64, high: f64, power: f32, release: f32) {
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(f32::total_cmp);
    let lo = percentile(&sorted, low);
    let scale = (percentile(&sorted, high) - lo).max(1e-6);
    let mut y = 0.0_f32;
    for value in values {
        let v = (((f64::from(*value) - lo) / scale).clamp(0.0, 1.0) as f32).powf(power);
        y = if v > y {
            v
        } else {
            y * release + v * (1.0 - release)
        };
        *value = y;
    }
}

/// Match Python's centered 4096-sample Hann FFT at 24 fps: 16 log bands (40..14000 Hz),
/// bass (28..150 Hz), percentile normalization, asymmetric release, kicks and 1440 px/s rings.
/// The golden fixture allows 0.0002 absolute error for envelopes and requires exact ring radii.
/// Empty/sub-frame sources return empty arrays; silence produces zero envelopes.
pub fn analyse(pcm: &PcmAudio) -> Result<AudioFeatures> {
    const WIN: usize = 4096;
    let n = pcm.samples.len() / SAMPLES_PER_FRAME as usize;
    if n == 0 {
        return Ok(AudioFeatures::default());
    }
    // Python multiplies float32 mono/window values before passing them to the FFT.
    let mono: Vec<f32> = pcm
        .samples
        .iter()
        .map(|s| (f32::from(s[0]) + f32::from(s[1])) / 65536.0)
        .collect();
    let window: Vec<f32> = (0..WIN)
        .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (WIN - 1) as f64).cos()) as f32)
        .collect();
    let edges: Vec<f64> = (0..=BANDS)
        .map(|b| 40.0 * (14000.0_f64 / 40.0).powf(b as f64 / BANDS as f64))
        .collect();
    let bins = |lo: f64, hi: f64| -> Vec<usize> {
        (0..=WIN / 2)
            .filter(|&i| {
                let hz = i as f64 * f64::from(SAMPLE_RATE) / WIN as f64;
                hz >= lo && hz < hi
            })
            .collect()
    };
    let selections: Vec<Vec<usize>> = edges.windows(2).map(|e| bins(e[0], e[1])).collect();
    let bass_bins = bins(28.0, 150.0);
    let mut bands = vec![[0.0; BANDS]; n];
    let mut bass = vec![0.0; n];
    let fft = FftPlanner::<f64>::new().plan_fft_forward(WIN);
    let mut buffer = vec![Complex::default(); WIN];
    let mut scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
    let mut magnitudes = vec![0.0; WIN / 2 + 1];
    for k in 0..n {
        for (i, value) in buffer.iter_mut().enumerate() {
            let index = (k * SAMPLES_PER_FRAME as usize + i).checked_sub(WIN / 2);
            let sample = index
                .and_then(|index| mono.get(index))
                .copied()
                .unwrap_or(0.0);
            *value = Complex::new(f64::from(sample * window[i]), 0.0);
        }
        fft.process_with_scratch(&mut buffer, &mut scratch);
        for (mag, value) in magnitudes.iter_mut().zip(&buffer) {
            *mag = value.norm();
        }
        let energy = |sel: &[usize]| -> f32 {
            (sel.iter().map(|&i| magnitudes[i]).sum::<f64>() / sel.len() as f64 * 8.0).ln_1p()
                as f32
        };
        for (b, sel) in selections.iter().enumerate() {
            bands[k][b] = energy(sel);
        }
        bass[k] = energy(&bass_bins);
    }
    for b in 0..BANDS {
        let mut column: Vec<f32> = bands.iter().map(|row| row[b]).collect();
        normalise_envelope(&mut column, 8.0, 98.5, 1.0, 0.8);
        for (row, value) in bands.iter_mut().zip(column) {
            row[b] = value;
        }
    }
    normalise_envelope(&mut bass, 30.0, 99.5, 1.4, 0.72);
    let flux: Vec<f32> = (0..n)
        .map(|i| (bass[i] - bass[i.saturating_sub(1)]).max(0.0))
        .collect();
    let mut kick = vec![0.0; n];
    let mut ring = vec![0.0; n];
    let (mut k, mut since, mut last) = (0.0, 0_u64, -99_i64);
    for i in 0..n {
        let local = &flux[i.saturating_sub(45)..n.min(i + 45)];
        let mean = local.iter().sum::<f32>() / local.len() as f32;
        let std =
            (local.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / local.len() as f32).sqrt();
        if flux[i] > mean + 1.5 * std + 0.03 && bass[i] > 0.25 && i as i64 - last > 6 {
            k = 1.0;
            last = i as i64;
            since = 0;
        } else {
            k *= 0.82;
            since += 1;
        }
        kick[i] = k;
        ring[i] = since as f32 * 1440.0 / FPS as f32;
    }
    AudioFeatures::new(bands, bass, kick, ring)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_short_and_silent_are_safe() {
        for count in [0, 1, 1999, 2000, 10000] {
            let f = analyse(&PcmAudio {
                samples: vec![[0, 0]; count],
            })
            .unwrap();
            assert_eq!(f.len(), count / 2000);
            assert!(f.bands().iter().flatten().all(|&v| v == 0.0));
            assert!(f.bass().iter().chain(f.kick()).all(|&v| v == 0.0));
            for (i, &ring) in f.ring().iter().enumerate() {
                assert_eq!(ring, (i + 1) as f32 * 60.0);
            }
        }
    }
}
