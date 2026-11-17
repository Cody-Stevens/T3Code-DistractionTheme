//! Explicit, opt-in measurements. Normal tests never open an audio device or invoke ffmpeg.
use dt_core::Clock;
use dt_media::{AudioOutput, AudioOutputConfig, CpalOutput, PcmAudio, analyse, decode_audio};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn benchmark() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("four-minutes.m4a");
    let status = Command::new("ffmpeg").args(["-v", "error", "-f", "lavfi", "-i",
        "aevalsrc=0.15*sin(2*PI*(40*t+10*t*t))+0.5*sin(2*PI*58*t)*exp(-35*mod(t\\,0.5)):s=48000:d=240",
        "-ac", "2", "-c:a", "aac", "-b:a", "128k", "-threads", "2"])
        .arg(&path).stdin(Stdio::null()).status()?;
    if !status.success() {
        return Err("ffmpeg generation failed".into());
    }
    for run in 1..=3 {
        let start = Instant::now();
        let pcm = decode_audio(&path, Path::new("ffmpeg"))?;
        let decode = start.elapsed();
        let start_analysis = Instant::now();
        let features = analyse(&pcm)?;
        let analysis = start_analysis.elapsed();
        println!(
            "run={run} PCM={} features={} decode_ms={:.3} analysis_ms={:.3} total_ms={:.3}",
            pcm.samples.len(),
            features.len(),
            decode.as_secs_f64() * 1000.0,
            analysis.as_secs_f64() * 1000.0,
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}

fn clock() -> Result<(), Box<dyn std::error::Error>> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no default output device")?;
    println!("device={}", device.name()?);
    let mut output = CpalOutput::open(&AudioOutputConfig::default())?;
    let pcm = PcmAudio {
        samples: (0..48000 * 9)
            .map(|i| {
                let value =
                    (328.0 * (std::f64::consts::TAU * 220.0 * f64::from(i) / 48000.0).sin()) as i16;
                [value, value]
            })
            .collect(),
    };
    output.set_volume(1.0)?;
    output.set_source(Some(Arc::new(pcm)))?;
    output.set_playing(true)?;
    thread::sleep(Duration::from_secs(1));
    let mut points = Vec::new();
    let start = Instant::now();
    let initial = output.media_time().as_secs_f64();
    let mut backwards = 0;
    let mut previous = initial;
    while start.elapsed() < Duration::from_secs(5) {
        let before = Instant::now();
        let media = output.media_time().as_secs_f64();
        let after = Instant::now();
        let wall = (before.duration_since(start).as_secs_f64()
            + after.duration_since(start).as_secs_f64())
            * 0.5;
        points.push((wall, media - initial));
        if media < previous {
            backwards += 1;
        }
        previous = media;
        thread::sleep(Duration::from_millis(2));
    }
    let n = points.len() as f64;
    let mean_x = points.iter().map(|p| p.0).sum::<f64>() / n;
    let mean_y = points.iter().map(|p| p.1).sum::<f64>() / n;
    let slope = points
        .iter()
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum::<f64>()
        / points
            .iter()
            .map(|(x, _)| (x - mean_x).powi(2))
            .sum::<f64>();
    let mut errors: Vec<f64> = points
        .iter()
        .map(|(x, y)| ((y - mean_y) - slope * (x - mean_x)) * 1000.0)
        .collect();
    let rms = (errors.iter().map(|e| e * e).sum::<f64>() / n).sqrt();
    errors.sort_by(f64::total_cmp);
    let peak_to_peak = errors.last().unwrap() - errors[0];
    let mut absolute: Vec<f64> = errors.iter().map(|e| e.abs()).collect();
    absolute.sort_by(f64::total_cmp);
    println!(
        "samples={} elapsed_s={:.3} latency_ms={:.3} slope={slope:.9} drift_ppm={:.3} jitter_rms_ms={rms:.6} jitter_p95_abs_ms={:.6} jitter_peak_to_peak_ms={peak_to_peak:.6} backwards={backwards}",
        points.len(),
        start.elapsed().as_secs_f64(),
        output.device_latency().as_secs_f64() * 1000.0,
        (slope - 1.0) * 1e6,
        absolute[absolute.len() * 95 / 100]
    );
    output.set_playing(false)?;
    let paused = output.media_time();
    thread::sleep(Duration::from_millis(50));
    assert_eq!(output.media_time(), paused);
    output.seek(Duration::from_secs(2))?;
    assert_eq!(output.media_time(), Duration::from_secs(2));
    output.stop()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("bench") => benchmark(),
        Some("clock") => clock(),
        _ => Err("usage: cargo run --release -p dt-media --example measure -- bench|clock".into()),
    }
}
