//! Device ownership, a preallocated callback command queue, and the audible sample clock.

use crate::PcmAudio;
use cpal::{
    FromSample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use dt_core::{Clock, Error, Result, constants::SAMPLE_RATE};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering::SeqCst},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Device selection for low-latency 48 kHz stereo output.
#[derive(Clone, Debug, Default)]
pub struct AudioOutputConfig {
    /// `None` selects the default device. Unsupported stereo/48 kHz devices return an error.
    pub device_name: Option<String>,
}

/// PCM output and controls; callbacks emit silence when paused/unloaded and zero-fill at EOF.
pub trait AudioOutput: Clock {
    /// Replace the source and reset to sample zero; acknowledgement precedes return.
    fn set_source(&mut self, pcm: Option<Arc<PcmAudio>>) -> Result<()>;
    /// Freeze/unfreeze audible media time and sample advancement.
    fn set_playing(&mut self, playing: bool) -> Result<()>;
    /// Emit silence and freeze the PCM cursor/clock until a release is scheduled by the player.
    /// Does not change playing state; source changes and seeks preserve the hold.
    fn hold(&mut self) -> Result<()>;
    /// Hold until this monotonic deadline, then allow PCM on the first callback at/after it.
    /// A past deadline releases immediately on acknowledgement; this never unpauses playback.
    fn release_at(&mut self, when: Instant) -> Result<()>;
    /// Whether an indefinite hold or a future release deadline is still active.
    fn gated(&self) -> bool;
    /// Seek to an absolute sample position clamped to source length, invalidating old clock state.
    fn seek(&mut self, position: Duration) -> Result<()>;
    /// Set gain in 0..=1.5, matching the player volume control.
    fn set_volume(&mut self, volume: f32) -> Result<()>;
    /// True after the final submitted sample has reached the audible clock.
    fn ended(&self) -> bool;
    /// Stop and join device/callback resources. Idempotent; also called on drop.
    fn stop(&mut self) -> Result<()>;
}

fn nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}
fn sample_ns(samples: u64) -> u64 {
    (u128::from(samples) * 1_000_000_000 / u128::from(SAMPLE_RATE)).min(u128::from(u64::MAX)) as u64
}
fn sample_position(position: Duration) -> u64 {
    (position.as_nanos() * u128::from(SAMPLE_RATE) / 1_000_000_000).min(u128::from(u64::MAX)) as u64
}

#[derive(Clone, Copy, Default)]
struct Snapshot {
    start: u64,
    submitted: u64,
    floor: u64,
    length: u64,
    wall: u64,
    latency: u64,
    running: bool,
    loaded: bool,
    gate_release: u64,
}

impl Snapshot {
    fn gated(self, now: u64) -> bool {
        self.gate_release > now
    }

    fn at(self, now: u64) -> Duration {
        let position = if self.running {
            sample_ns(self.start)
                .saturating_add(now.saturating_sub(self.wall))
                .saturating_sub(self.latency)
                .clamp(sample_ns(self.floor), sample_ns(self.submitted))
        } else {
            sample_ns(self.start)
        };
        Duration::from_nanos(position.min(sample_ns(self.length)))
    }
}

#[derive(Default)]
struct Shared {
    version: AtomicU64,
    fields: [AtomicU64; 9],
    ack: AtomicU64,
    failed: AtomicBool,
}

impl Shared {
    fn publish(&self, s: Snapshot) {
        self.version.fetch_add(1, SeqCst);
        for (field, value) in self.fields.iter().zip([
            s.start,
            s.submitted,
            s.floor,
            s.length,
            s.wall,
            s.latency,
            u64::from(s.running),
            u64::from(s.loaded),
            s.gate_release,
        ]) {
            field.store(value, SeqCst);
        }
        self.version.fetch_add(1, SeqCst);
    }

    fn snapshot(&self) -> Snapshot {
        loop {
            let version = self.version.load(SeqCst);
            if version & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let [
                start,
                submitted,
                floor,
                length,
                wall,
                latency,
                running,
                loaded,
                gate_release,
            ] = std::array::from_fn(|i| self.fields[i].load(SeqCst));
            if version == self.version.load(SeqCst) {
                return Snapshot {
                    start,
                    submitted,
                    floor,
                    length,
                    wall,
                    latency,
                    running: running != 0,
                    loaded: loaded != 0,
                    gate_release,
                };
            }
        }
    }
}

enum Control {
    Source(Option<Arc<PcmAudio>>),
    Playing(bool),
    Seek(u64),
    Volume(f32),
    Hold,
    ReleaseAt(u64),
}

struct Commands {
    producer: Producer<(u64, Control)>,
    retired: Consumer<Arc<PcmAudio>>,
    serial: u64,
}

struct Callback {
    source: Option<Arc<PcmAudio>>,
    cursor: u64,
    floor: u64,
    playing: bool,
    volume: f32,
    gate_release: Option<u64>,
    commands: Consumer<(u64, Control)>,
    retired: Producer<Arc<PcmAudio>>,
    shared: Arc<Shared>,
}

impl Callback {
    fn render<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T], now: u64, latency: u64) {
        let mut ack = None;
        let mut changed = false;
        while let Ok((serial, control)) = self.commands.pop() {
            match control {
                Control::Source(source) => {
                    if let Some(old) = std::mem::replace(&mut self.source, source) {
                        // Each synchronous command drains retired sources before/after its ack.
                        // Capacity exceeds the entire command queue, so this cannot fill.
                        if self.retired.push(old).is_err() {
                            self.shared.failed.store(true, SeqCst);
                        }
                    }
                    self.cursor = 0;
                    self.floor = 0;
                    changed = true;
                }
                Control::Playing(playing) => {
                    if self.playing != playing {
                        // Held/paused cursors are already frozen at an exact sample boundary.
                        if self.playing && self.gate_release.is_none() {
                            self.cursor = sample_position(self.shared.snapshot().at(now));
                        }
                        self.floor = self.cursor;
                        self.playing = playing;
                        changed = true;
                    }
                }
                Control::Seek(position) => {
                    self.cursor =
                        position.min(self.source.as_ref().map_or(0, |s| s.samples.len() as u64));
                    self.floor = self.cursor;
                    changed = true;
                }
                Control::Volume(volume) => self.volume = volume,
                Control::Hold => {
                    self.gate_release = Some(u64::MAX);
                    self.floor = self.cursor;
                    changed = true;
                }
                Control::ReleaseAt(deadline) => {
                    self.gate_release = Some(deadline);
                    self.floor = self.cursor;
                    changed = true;
                }
            }
            ack = Some(serial);
        }
        if self.gate_release.is_some_and(|deadline| now >= deadline) {
            self.gate_release = None;
            self.floor = self.cursor;
            changed = true;
        }
        out.fill(T::from_sample(0.0));
        let length = self.source.as_ref().map_or(0, |s| s.samples.len() as u64);
        let start = self.cursor;
        if self.playing
            && self.gate_release.is_none()
            && let Some(pcm) = &self.source
        {
            for (dest, sample) in out
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(pcm.samples[start as usize..].iter())
            {
                for channel in 0..2 {
                    // Match NumPy's float multiplication, truncation and int16 wrapping.
                    let scaled =
                        (f64::from(sample[channel]) * f64::from(self.volume)) as i64 as i16;
                    dest[channel] = T::from_sample(f32::from(scaled) / 32768.0);
                }
                self.cursor += 1;
            }
        }
        // Preserve the final callback anchor while silence drains; otherwise EOF stalls one
        // device latency before the last sample forever. Stalls cannot run past submitted PCM.
        if changed || self.cursor != start {
            self.shared.publish(Snapshot {
                start,
                submitted: self.cursor,
                floor: self.floor,
                length,
                wall: now,
                latency,
                running: self.playing && self.gate_release.is_none() && self.source.is_some(),
                loaded: self.source.is_some(),
                gate_release: self.gate_release.unwrap_or(0),
            });
        } else if ack.is_some() {
            // A no-op control still publishes device latency before acknowledging it, while
            // preserving the final clock anchor when no new PCM was submitted.
            let mut snapshot = self.shared.snapshot();
            snapshot.latency = latency;
            self.shared.publish(snapshot);
        }
        if let Some(serial) = ack {
            self.shared.ack.store(serial, SeqCst);
        }
    }
}

fn stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut callback: Callback,
    origin: Instant,
) -> Result<cpal::Stream> {
    let errors = Arc::clone(&callback.shared);
    device
        .build_output_stream::<T, _, _>(
            config,
            move |out, info| {
                let timestamp = info.timestamp();
                let latency = timestamp
                    .playback
                    .duration_since(&timestamp.callback)
                    .unwrap_or_default();
                callback.render(out, nanos(origin.elapsed()), nanos(latency));
            },
            move |_| {
                errors.failed.store(true, SeqCst);
            },
            None,
        )
        .map_err(|_| Error::Protocol("cpal build output stream"))
}

fn open_stream(
    config: AudioOutputConfig,
    callback: Callback,
    origin: Instant,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = if let Some(name) = config.device_name {
        host.output_devices()
            .map_err(|_| Error::Protocol("cpal enumerate outputs"))?
            .find(|d| d.name().is_ok_and(|n| n == name))
    } else {
        host.default_output_device()
    }
    .ok_or(Error::Protocol("no matching audio output device"))?;
    let ranges = device
        .supported_output_configs()
        .map_err(|_| Error::Protocol("cpal output formats"))?;
    let selected = ranges
        .filter(|r| {
            r.channels() == 2
                && r.min_sample_rate().0 <= SAMPLE_RATE
                && r.max_sample_rate().0 >= SAMPLE_RATE
        })
        .filter(|r| {
            matches!(
                r.sample_format(),
                SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
            )
        })
        .min_by_key(|r| {
            if r.sample_format() == SampleFormat::F32 {
                0
            } else {
                1
            }
        })
        .ok_or(Error::InvalidInput(
            "device requires unsupported stereo/48 kHz format",
        ))?
        .with_sample_rate(cpal::SampleRate(SAMPLE_RATE));
    let mut stream_config = selected.config();
    // About 10 ms where the backend exposes a range. WASAPI's fixed hardware period wins.
    if let cpal::SupportedBufferSize::Range { min, max } = selected.buffer_size() {
        stream_config.buffer_size = cpal::BufferSize::Fixed(480_u32.clamp(*min, *max));
    }
    match selected.sample_format() {
        SampleFormat::F32 => stream::<f32>(&device, &stream_config, callback, origin),
        SampleFormat::I16 => stream::<i16>(&device, &stream_config, callback, origin),
        SampleFormat::U16 => stream::<u16>(&device, &stream_config, callback, origin),
        _ => Err(Error::InvalidInput("unsupported output sample format")),
    }
}

/// cpal/WASAPI output with a thread-safe clock/control handle and owned device thread.
/// Callback work uses only PCM reads, arithmetic, atomics and preallocated SPSC queues.
/// Retired PCM is freed by the control thread. Device errors make later controls fail.
pub struct CpalOutput {
    origin: Instant,
    shared: Arc<Shared>,
    commands: Mutex<Option<Commands>>,
    shutdown: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl CpalOutput {
    /// Latest backend-estimated delay from callback time to playback time.
    /// This excludes any hardware delay the backend cannot report.
    pub fn device_latency(&self) -> Duration {
        Duration::from_nanos(self.shared.snapshot().latency)
    }

    /// Open a running silence stream. PCM starts only after `set_source` and `set_playing(true)`.
    pub fn open(config: &AudioOutputConfig) -> Result<Self> {
        let origin = Instant::now();
        let shared = Arc::new(Shared::default());
        let (producer, consumer) = RingBuffer::new(32);
        let (retire_producer, retire_consumer) = RingBuffer::new(64);
        let callback = Callback {
            source: None,
            cursor: 0,
            floor: 0,
            playing: false,
            volume: 0.8,
            gate_release: None,
            commands: consumer,
            retired: retire_producer,
            shared: Arc::clone(&shared),
        };
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (shutdown, stop) = mpsc::channel();
        let config = config.clone();
        let worker = thread::Builder::new()
            .name("dt-audio".into())
            .spawn(move || {
                match open_stream(config, callback, origin).and_then(|stream| {
                    stream
                        .play()
                        .map_err(|_| Error::Protocol("cpal start stream"))?;
                    Ok(stream)
                }) {
                    Ok(stream) => {
                        let _ = ready_tx.send(Ok(()));
                        let _ = stop.recv();
                        drop(stream);
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                origin,
                shared,
                commands: Mutex::new(Some(Commands {
                    producer,
                    retired: retire_consumer,
                    serial: 0,
                })),
                shutdown: Some(shutdown),
                worker: Some(worker),
            }),
            result => {
                let _ = worker.join();
                result.unwrap_or(Err(Error::Protocol("audio device thread exited")))?;
                unreachable!()
            }
        }
    }

    fn control(&mut self, control: Control) -> Result<()> {
        if self.worker.is_none() || self.shared.failed.load(SeqCst) {
            return Err(Error::Protocol("audio stream stopped or failed"));
        }
        let mut commands = self
            .commands
            .lock()
            .map_err(|_| Error::Protocol("audio control lock"))?;
        let commands = commands
            .as_mut()
            .ok_or(Error::Protocol("audio stream stopped"))?;
        while commands.retired.pop().is_ok() {}
        commands.serial += 1;
        let serial = commands.serial;
        commands
            .producer
            .push((serial, control))
            .map_err(|_| Error::Protocol("audio command queue full"))?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.shared.ack.load(SeqCst) < serial {
            if self.shared.failed.load(SeqCst) || Instant::now() >= deadline {
                return Err(Error::Protocol(
                    "audio callback did not acknowledge control",
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
        while commands.retired.pop().is_ok() {}
        Ok(())
    }
}

impl Clock for CpalOutput {
    fn media_time(&self) -> Duration {
        self.shared.snapshot().at(nanos(self.origin.elapsed()))
    }
}

impl AudioOutput for CpalOutput {
    fn set_source(&mut self, pcm: Option<Arc<PcmAudio>>) -> Result<()> {
        self.control(Control::Source(pcm))
    }
    fn set_playing(&mut self, playing: bool) -> Result<()> {
        self.control(Control::Playing(playing))
    }
    fn hold(&mut self) -> Result<()> {
        self.control(Control::Hold)
    }
    fn release_at(&mut self, when: Instant) -> Result<()> {
        self.control(Control::ReleaseAt(nanos(
            when.saturating_duration_since(self.origin),
        )))
    }
    fn gated(&self) -> bool {
        self.shared.snapshot().gated(nanos(self.origin.elapsed()))
    }
    fn seek(&mut self, position: Duration) -> Result<()> {
        self.control(Control::Seek(sample_position(position)))
    }
    fn set_volume(&mut self, volume: f32) -> Result<()> {
        if !volume.is_finite() || !(0.0..=1.5).contains(&volume) {
            return Err(Error::InvalidInput("volume must be finite and in 0..=1.5"));
        }
        self.control(Control::Volume(volume))
    }
    fn ended(&self) -> bool {
        let s = self.shared.snapshot();
        s.loaded
            && s.submitted == s.length
            && s.at(nanos(self.origin.elapsed())) >= Duration::from_nanos(sample_ns(s.length))
    }
    fn stop(&mut self) -> Result<()> {
        if self.worker.is_none() {
            return Ok(());
        }
        let _ = self.set_playing(false);
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| Error::Protocol("audio device thread panicked"))?;
        }
        let mut commands = self
            .commands
            .lock()
            .map_err(|_| Error::Protocol("audio control lock"))?;
        // Drop the producer as well: a failed callback may have left a queued source unconsumed.
        commands.take();
        Ok(())
    }
}

impl Drop for CpalOutput {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> (Callback, Producer<(u64, Control)>, Consumer<Arc<PcmAudio>>) {
        let (producer, commands) = RingBuffer::new(32);
        let (retired, consumer) = RingBuffer::new(64);
        (
            Callback {
                source: None,
                cursor: 0,
                floor: 0,
                playing: false,
                volume: 1.0,
                gate_release: None,
                commands,
                retired,
                shared: Arc::default(),
            },
            producer,
            consumer,
        )
    }

    #[test]
    fn no_op_pause_acknowledges_the_initial_device_latency() {
        let (mut cb, mut commands, _) = fake();
        commands.push((1, Control::Playing(false))).ok().unwrap();
        let mut out = [1_i16; 960];
        cb.render(&mut out, 1_000_000_000, 200_000_000);
        assert_eq!(cb.shared.ack.load(SeqCst), 1);
        let snapshot = cb.shared.snapshot();
        assert_eq!(snapshot.latency, 200_000_000);
        assert_eq!(snapshot.at(2_000_000_000), Duration::ZERO);
        assert!(!snapshot.running);
        assert_eq!(out, [0; 960]);
    }

    #[test]
    fn latency_interpolation_clamps_and_freezes() {
        let s = Snapshot {
            start: 48000,
            submitted: 48960,
            floor: 0,
            length: 96000,
            wall: 1_000_000_000,
            latency: 10_000_000,
            running: true,
            loaded: true,
            gate_release: 0,
        };
        assert_eq!(s.at(s.wall), Duration::from_millis(990));
        assert_eq!(s.at(s.wall + 15_000_000), Duration::from_millis(1005));
        assert_eq!(s.at(s.wall + 1_000_000_000), Duration::from_millis(1020));
        assert_eq!(
            Snapshot {
                running: false,
                ..s
            }
            .at(u64::MAX),
            Duration::from_secs(1)
        );
        assert_eq!(
            Snapshot {
                start: 0,
                floor: 0,
                ..s
            }
            .at(s.wall),
            Duration::ZERO
        );
        assert_eq!(
            Snapshot { floor: 48000, ..s }.at(s.wall),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn callback_partial_eof_pause_seek_and_source_retirement() {
        let (mut cb, mut commands, mut retired) = fake();
        let source = Arc::new(PcmAudio {
            samples: vec![[16384, -16384]; 600],
        });
        commands
            .push((1, Control::Source(Some(Arc::clone(&source)))))
            .ok()
            .unwrap();
        let mut out = [1.0_f32; 960];
        cb.render(&mut out, 0, 10_000_000);
        assert!(out.iter().all(|&v| v == 0.0));
        commands.push((2, Control::Playing(true))).ok().unwrap();
        cb.render(&mut out, 10_000_000, 10_000_000);
        assert_eq!(&out[..4], &[0.5, -0.5, 0.5, -0.5]);
        assert_eq!(cb.shared.snapshot().at(10_000_000), Duration::ZERO);
        cb.render(&mut out, 20_000_000, 10_000_000);
        assert!(out[240..].iter().all(|&v| v == 0.0));
        assert_eq!(cb.cursor, 600);
        let final_anchor = cb.shared.snapshot().wall;
        cb.render(&mut out, 30_000_000, 10_000_000);
        assert_eq!(cb.shared.snapshot().wall, final_anchor);
        commands.push((3, Control::Volume(1.0))).ok().unwrap();
        cb.render(&mut out, 40_000_000, 10_000_000);
        assert_eq!(cb.shared.snapshot().wall, final_anchor);
        assert_eq!(
            cb.shared.snapshot().at(100_000_000),
            Duration::from_micros(12500)
        );
        commands.push((4, Control::Seek(96))).ok().unwrap();
        cb.render(&mut out, 100_000_000, 10_000_000);
        assert_eq!(
            cb.shared.snapshot().at(100_000_000),
            Duration::from_millis(2)
        );
        commands.push((5, Control::Playing(false))).ok().unwrap();
        cb.render(&mut out, 105_000_000, 10_000_000);
        let paused = cb.shared.snapshot();
        assert_eq!(paused.at(200_000_000), paused.at(105_000_000));
        assert!(out.iter().all(|&v| v == 0.0));
        commands.push((6, Control::Source(None))).ok().unwrap();
        cb.render(&mut out, 210_000_000, 0);
        assert_eq!(Arc::strong_count(&source), 2);
        drop(retired.pop().unwrap());
        assert_eq!(Arc::strong_count(&source), 1);
        assert_eq!(cb.shared.snapshot().at(220_000_000), Duration::ZERO);
    }

    #[test]
    fn callback_gain_matches_int16_truncation_and_wrapping() {
        let (mut cb, mut commands, _) = fake();
        cb.source = Some(Arc::new(PcmAudio {
            samples: vec![[32767, -32768], [3, -3]],
        }));
        cb.playing = true;
        commands.push((1, Control::Volume(1.5))).ok().unwrap();
        let mut out = [0_i16; 8];
        cb.render(&mut out, 0, 0);
        assert_eq!(out, [-16386, 16384, 4, -4, 0, 0, 0, 0]);
    }

    fn apply(
        cb: &mut Callback,
        commands: &mut Producer<(u64, Control)>,
        control: Control,
        now: u64,
    ) -> [i16; 960] {
        let serial = cb.shared.ack.load(SeqCst) + 1;
        commands.push((serial, control)).ok().unwrap();
        let mut out = [1; 960];
        cb.render(&mut out, now, 10_000_000);
        assert_eq!(cb.shared.ack.load(SeqCst), serial);
        out
    }

    #[test]
    fn hold_silences_output_and_freezes_clock_until_the_release_callback() {
        let (mut cb, mut commands, _) = fake();
        cb.source = Some(Arc::new(PcmAudio {
            samples: vec![[16384, -16384]; 48000],
        }));
        cb.playing = true;
        assert_eq!(apply(&mut cb, &mut commands, Control::Hold, 0), [0; 960]);
        let mut out = [1_i16; 960];
        cb.render(&mut out, 1_000_000_000, 10_000_000);
        assert_eq!(out, [0; 960]);
        assert_eq!(cb.cursor, 0);
        let held = cb.shared.snapshot();
        assert!(held.gated(1_000_000_000));
        assert_eq!(held.at(1_000_000_000), Duration::ZERO);
        assert_eq!(held.at(60_000_000_000), Duration::ZERO);

        assert_eq!(
            apply(
                &mut cb,
                &mut commands,
                Control::ReleaseAt(2_000_000_000),
                1_500_000_000
            ),
            [0; 960]
        );
        cb.render(&mut out, 1_999_999_999, 10_000_000);
        assert_eq!(out, [0; 960]);
        assert_eq!(cb.cursor, 0);
        let scheduled = cb.shared.snapshot();
        assert!(scheduled.gated(1_999_999_999));
        assert!(!scheduled.gated(2_000_000_000));
        // Expiring a deadline cannot invent samples before the callback submits any.
        assert_eq!(scheduled.at(2_005_000_000), Duration::ZERO);
        cb.render(&mut out, 2_000_000_000, 10_000_000);
        assert_eq!(&out[..4], &[16384, -16384, 16384, -16384]);
        assert_eq!(cb.cursor, 480);
        let released = cb.shared.snapshot();
        assert!(!released.gated(2_000_000_000));
        assert_eq!(released.at(2_000_000_000), Duration::ZERO);
        assert_eq!(released.at(2_015_000_000), Duration::from_millis(5));
    }

    #[test]
    fn source_seek_and_resume_preserve_hold_and_release_does_not_unpause() {
        let (mut cb, mut commands, _) = fake();
        apply(&mut cb, &mut commands, Control::Hold, 0);
        let source = Arc::new(PcmAudio {
            samples: vec![[123, -123]; 48000 * 4],
        });
        assert_eq!(
            apply(
                &mut cb,
                &mut commands,
                Control::Source(Some(source)),
                10_000_000
            ),
            [0; 960]
        );
        assert_eq!(
            apply(&mut cb, &mut commands, Control::Seek(96000), 20_000_000),
            [0; 960]
        );
        assert_eq!(
            apply(&mut cb, &mut commands, Control::Playing(true), 30_000_000),
            [0; 960]
        );
        assert_eq!(cb.cursor, 96000);
        assert_eq!(
            cb.shared.snapshot().at(1_000_000_000),
            Duration::from_secs(2)
        );
        apply(
            &mut cb,
            &mut commands,
            Control::ReleaseAt(100_000_000),
            40_000_000,
        );
        apply(&mut cb, &mut commands, Control::Playing(false), 50_000_000);
        let mut out = [1_i16; 960];
        cb.render(&mut out, 100_000_000, 10_000_000);
        assert_eq!(out, [0; 960]);
        assert!(!cb.playing);
        assert!(!cb.shared.snapshot().gated(100_000_000));
        assert_eq!(cb.cursor, 96000);
        assert_eq!(
            cb.shared.snapshot().at(1_000_000_000),
            Duration::from_secs(2)
        );

        // A new resume hold can be installed before unpausing without losing the seek target.
        apply(&mut cb, &mut commands, Control::Hold, 110_000_000);
        assert_eq!(
            apply(&mut cb, &mut commands, Control::Playing(true), 120_000_000),
            [0; 960]
        );
        assert_eq!(cb.cursor, 96000);
        let resumed = apply(
            &mut cb,
            &mut commands,
            Control::ReleaseAt(130_000_000),
            130_000_000,
        );
        assert_eq!(&resumed[..4], &[123, -123, 123, -123]);
        assert_eq!(cb.cursor, 96480);
        assert_eq!(
            cb.shared.snapshot().at(145_000_000),
            Duration::from_millis(2005)
        );
        apply(&mut cb, &mut commands, Control::Hold, 150_000_000);
        let frozen_cursor = cb.cursor;
        cb.render(&mut out, 2_000_000_000, 10_000_000);
        assert_eq!(out, [0; 960]);
        assert_eq!(cb.cursor, frozen_cursor);
        assert_eq!(
            cb.shared.snapshot().at(3_000_000_000),
            Duration::from_millis(2010)
        );
    }

    #[test]
    fn holds_cancel_old_deadlines_and_past_deadlines_release_immediately() {
        let (mut cb, mut commands, _) = fake();
        cb.source = Some(Arc::new(PcmAudio {
            samples: vec![[321, -321]; 48000],
        }));
        cb.playing = true;
        apply(&mut cb, &mut commands, Control::Hold, 0);
        apply(
            &mut cb,
            &mut commands,
            Control::ReleaseAt(50_000_000),
            10_000_000,
        );
        apply(
            &mut cb,
            &mut commands,
            Control::ReleaseAt(100_000_000),
            20_000_000,
        );
        let mut out = [1_i16; 960];
        cb.render(&mut out, 50_000_000, 10_000_000);
        assert_eq!(out, [0; 960]);
        assert!(cb.shared.snapshot().gated(50_000_000));
        apply(&mut cb, &mut commands, Control::Hold, 60_000_000);
        cb.render(&mut out, 200_000_000, 10_000_000);
        assert_eq!(out, [0; 960]);
        assert_eq!(cb.cursor, 0);
        assert!(cb.shared.snapshot().gated(200_000_000));
        let released = apply(
            &mut cb,
            &mut commands,
            Control::ReleaseAt(100_000_000),
            210_000_000,
        );
        assert_eq!(&released[..4], &[321, -321, 321, -321]);
        assert_eq!(cb.cursor, 480);
        assert!(!cb.shared.snapshot().gated(210_000_000));
    }
}
