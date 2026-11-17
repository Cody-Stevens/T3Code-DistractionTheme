//! Fake-clock replay of measured encode, quiet-write, image-load and audio-output timings.
use dt_core::TrackInfo;
use dt_t3::favicon::*;
use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value)
}
fn track(length: u64) -> TrackInfo {
    let mut track = TrackInfo::new("synthetic");
    track.id = "synthetic".into();
    track.path = Some(PathBuf::from("synthetic.mp4"));
    track.frames = length * 24;
    track
}
fn config() -> FaviconConfig {
    FaviconConfig {
        clips_dir: std::env::temp_dir().join("fake-clips-no-io"),
        project_id: "project-a".into(),
        deck_b_project_id: Some("project-b".into()),
        run_id: "fake-run".into(),
        show_latency: seconds(0.5),
        first_clip: seconds(10.0),
        clip_seconds: seconds(22.0),
        gate: true,
    }
}
struct Work {
    begin: f64,
    due: f64,
    job: ClipJob,
    started: bool,
}
struct Publication {
    wall: f64,
    media: f64,
    deck: Deck,
    path: PathBuf,
}
struct Simulation {
    scheduler: FaviconScheduler,
    epoch: Instant,
    wall: f64,
    media: f64,
    jumps: u64,
    worker_free: f64,
    encodes: Vec<Work>,
    events: Vec<FaviconEvent>,
    grant_due: Option<f64>,
    grant: Option<Instant>,
    jobs: Vec<ClipJob>,
    publishes: Vec<Publication>,
    requests: Vec<f64>,
    flips: Vec<(f64, Deck)>,
    allowed: Vec<PathBuf>,
    playing: bool,
    enabled: bool,
    delay_one: bool,
    fail_publish: bool,
    quiet_build: f64,
    grant_quiet: bool,
    output_latency: f64,
    audible_at: f64,
    holds: usize,
    replacements: usize,
    blanks: usize,
}
impl Simulation {
    fn new() -> Self {
        Self::with_config(config())
    }
    fn with_config(config: FaviconConfig) -> Self {
        Self {
            scheduler: FaviconScheduler::new(config),
            epoch: Instant::now(),
            wall: 0.0,
            media: 0.0,
            jumps: 0,
            worker_free: 0.0,
            encodes: Vec::new(),
            events: Vec::new(),
            grant_due: None,
            grant: None,
            jobs: Vec::new(),
            publishes: Vec::new(),
            requests: Vec::new(),
            flips: Vec::new(),
            allowed: Vec::new(),
            playing: true,
            enabled: true,
            delay_one: false,
            fail_publish: false,
            quiet_build: 0.6,
            grant_quiet: true,
            output_latency: 0.1,
            audible_at: 0.0,
            holds: 0,
            replacements: 0,
            blanks: 0,
        }
    }
    fn music_plays(&self) -> bool {
        self.playing && self.wall + 1e-8 >= self.audible_at
    }
    fn step(&mut self, track: Option<&TrackInfo>) -> Vec<FaviconAction> {
        if self.grant_due.is_some_and(|due| self.wall + 1e-8 >= due) {
            self.grant = Some(self.epoch + seconds(self.grant_due.take().unwrap()));
        }
        for work in &mut self.encodes {
            if !work.started && work.begin <= self.wall + 1e-8 {
                work.started = true;
                self.events
                    .push(FaviconEvent::EncodeStarted(work.job.clone()));
            }
            if work.due <= self.wall + 1e-8 {
                self.events.push(FaviconEvent::Encoded(ReadyClip {
                    job: work.job.clone(),
                    frames: (work.job.duration.as_secs_f64() * 24.0).round() as u64,
                }));
            }
        }
        self.encodes.retain(|work| work.due > self.wall + 1e-8);
        let mut all = Vec::new();
        // Acknowledge synchronous effects at the same clock instant, as Python's I/O does.
        for _ in 0..10 {
            let actions = self
                .scheduler
                .tick(FaviconTick {
                    track,
                    media_time: seconds(self.media),
                    jumps: self.jumps,
                    wall_time: self.epoch + seconds(self.wall),
                    playing: self.playing,
                    enabled: self.enabled,
                    output_latency: seconds(self.output_latency),
                    quiet_granted_at: self.grant,
                    events: std::mem::take(&mut self.events),
                })
                .unwrap();
            for action in &actions {
                match action {
                    FaviconAction::Encode(job) => {
                        assert!(self.encodes.len() < 2, "at most two encodes in flight");
                        let cost = 0.3
                            + job.duration.as_secs_f64()
                                * if job.speed == 10 { 0.15 } else { 0.22 };
                        let begin = self.wall.max(self.worker_free);
                        let mut due = begin + cost;
                        if self.delay_one {
                            due += 20.0;
                            self.delay_one = false;
                        }
                        self.worker_free = due;
                        let started = begin <= self.wall + 1e-8;
                        if started {
                            self.events.push(FaviconEvent::EncodeStarted(job.clone()));
                        }
                        self.encodes.push(Work {
                            begin,
                            due,
                            job: job.clone(),
                            started,
                        });
                        self.jobs.push(job.clone());
                    }
                    FaviconAction::HoldAudio {
                        request_id,
                        deadline,
                    } => {
                        self.holds += 1;
                        self.audible_at =
                            deadline.duration_since(self.epoch).as_secs_f64() + self.output_latency;
                        self.events.push(FaviconEvent::AudioHeld {
                            request_id: *request_id,
                            media_time: seconds(self.media),
                        });
                    }
                    FaviconAction::ReleaseAudio { at } => {
                        self.audible_at = at
                            .map_or(self.wall, |at| at.duration_since(self.epoch).as_secs_f64())
                            + self.output_latency;
                    }
                    FaviconAction::RequestQuiet => {
                        assert!(self.grant_due.is_none());
                        self.requests.push(self.media);
                        if self.grant_quiet {
                            self.grant_due = Some(self.wall + self.quiet_build);
                        }
                    }
                    FaviconAction::Publish {
                        deck,
                        project_id,
                        path,
                    } => {
                        assert_eq!(
                            project_id,
                            if *deck == Deck::A {
                                "project-a"
                            } else {
                                "project-b"
                            }
                        );
                        let elapsed =
                            (self.epoch + seconds(self.wall)).duration_since(self.grant.unwrap());
                        assert!(
                            elapsed >= QUIET_SETTLE
                                && elapsed < QUIET_SETTLE + QUIET_HOLD - seconds(1.0)
                        );
                        let job = self.jobs.iter().find(|j| j.output == *path).unwrap();
                        assert!(
                            !self.encodes.iter().any(|w| w.job == *job),
                            "never publish an unfinished encode"
                        );
                        if self.fail_publish {
                            self.fail_publish = false;
                            self.events.push(FaviconEvent::PublishFailed(path.clone()));
                        } else {
                            self.publishes.push(Publication {
                                wall: self.wall,
                                media: self.media,
                                deck: *deck,
                                path: path.clone(),
                            });
                            self.events.push(FaviconEvent::Published(path.clone()));
                        }
                    }
                    FaviconAction::SetFront(Some(deck)) => self.flips.push((self.media, *deck)),
                    FaviconAction::SetAllowedFiles(files) => self.allowed = files.clone(),
                    FaviconAction::CancelPending => {
                        // Future.cancel only removes queued work. Running work needs replacement.
                        self.encodes.retain(|w| w.started);
                        self.worker_free = self.encodes.last().map_or(self.wall, |w| w.due);
                    }
                    FaviconAction::ReplaceEncoder => {
                        self.replacements += 1;
                        self.encodes.clear();
                        self.worker_free = self.wall;
                    }
                    FaviconAction::Blank { .. } => self.blanks += 1,
                    FaviconAction::Cleanup { keep, .. } => assert_eq!(*keep, 30),
                    _ => (),
                }
            }
            all.extend(actions);
            if self.events.is_empty() {
                return all;
            }
        }
        panic!("effects did not settle");
    }
    fn advance(&mut self) {
        let next = ((self.wall * 10.0).round() + 1.0) / 10.0;
        if self.playing {
            let delta = (next - self.wall.max(self.audible_at)).max(0.0);
            // Check both ends of every advancing interval, including the first audible sample.
            if delta > 1e-8 && self.enabled && !self.publishes.is_empty() {
                self.assert_interval_covered(self.media, self.media + delta, next - delta);
            }
            self.media = ((self.media + delta) * 1e9).round() / 1e9;
        }
        self.wall = next;
    }
    fn run_until(&mut self, track: &TrackInfo, media: f64) {
        let deadline = self.wall + (media - self.media).max(0.0) + 100.0;
        while self.media + 1e-8 < media {
            assert!(self.wall < deadline, "clock remained held");
            self.step(Some(track));
            self.advance();
        }
        self.step(Some(track));
    }
    fn assert_interval_covered(&self, begin: f64, end: f64, wall: f64) {
        let clip = self
            .scheduler
            .front_clip()
            .expect("music needs a front clip");
        assert!(
            clip.job.start.as_secs_f64() <= begin + 1e-8 && clip.end().as_secs_f64() + 1e-8 >= end,
            "front {}..{} does not cover {begin}..{end}",
            clip.job.start.as_secs_f64(),
            clip.end().as_secs_f64()
        );
        let publish = self
            .publishes
            .iter()
            .find(|p| p.path == clip.job.output)
            .unwrap();
        assert!(
            wall + 1e-8 >= publish.wall + 0.5,
            "picture must load before music"
        );
        assert!(
            self.allowed
                .contains(&PathBuf::from(clip.job.output.file_name().unwrap()))
        );
    }
    fn assert_covered(&self) {
        self.assert_interval_covered(self.media, self.media, self.wall);
    }
    fn seek(&mut self, media: f64) {
        self.media = media;
        self.jumps += 1;
    }
}

#[test]
fn start_and_steady_playback_cover_every_audible_interval() {
    let track = track(400);
    let mut sim = Simulation::new();
    let actions = sim.step(Some(&track));
    assert!(matches!(actions[0], FaviconAction::HoldAudio { .. }));
    assert_eq!(sim.jobs[0].start, Duration::ZERO);
    assert_eq!(sim.jobs[0].duration, seconds(10.0));
    assert_eq!(sim.jobs[0].speed, 10);
    assert!(!sim.music_plays());
    let mut samples = 0;
    while sim.media < 300.0 {
        assert!(sim.wall < 400.0);
        sim.advance();
        sim.step(Some(&track));
        if sim.music_plays() {
            sim.assert_covered();
            samples += 1;
        }
    }
    assert!(samples >= 3000);
    assert_eq!(sim.holds, 1);
    assert_eq!(sim.blanks, 0);
    assert!(sim.jobs.iter().all(|j| j.duration <= seconds(25.0)));
    let lengths: Vec<_> = sim
        .jobs
        .iter()
        .take(4)
        .map(|j| j.duration.as_secs_f64())
        .collect();
    println!(
        "coverage: {samples}/{samples} audible samples plus every continuous interval; first durations {lengths:?}"
    );
    // Golden values from AST-only replay of the actual Python scheduler.
    for (job, (start, duration)) in sim.jobs.iter().zip([
        (0.0, 10.0),
        (7.0, 10.839160839),
        (14.839160839, 25.0),
        (36.839160839, 25.0),
    ]) {
        assert!((job.start.as_secs_f64() - start).abs() < 1e-8);
        assert!((job.duration.as_secs_f64() - duration).abs() < 1e-8);
    }
    assert_eq!(
        sim.publishes
            .iter()
            .take(4)
            .map(|p| p.media)
            .collect::<Vec<_>>(),
        [0.0, 6.4, 14.3, 36.3]
    );
    assert_eq!(
        &sim.flips[..4],
        [
            (0.0, Deck::A),
            (7.6, Deck::B),
            (15.5, Deck::A),
            (37.5, Deck::B)
        ]
    );
    assert_eq!(&sim.requests[..4], [0.0, 5.3, 13.1, 35.1]);
    for pair in sim.publishes.windows(2) {
        assert_ne!(pair[0].deck, pair[1].deck);
    }
    for p in &sim.publishes {
        let job = sim.jobs.iter().find(|j| j.output == p.path).unwrap();
        if p.media > 0.0 {
            assert!((p.media + 0.5 - job.start.as_secs_f64()).abs() <= 0.25);
        }
    }
    assert_eq!(
        sim.jobs
            .iter()
            .map(|j| &j.output)
            .collect::<HashSet<_>>()
            .len(),
        sim.jobs.len()
    );
}

#[test]
fn in_range_seek_and_track_change_restart_without_blank_publications() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.run_until(&track, 12.0);
    let old = sim.scheduler.front_clip().unwrap().job.clone();
    assert!(
        old.start <= seconds(10.0) && seconds(10.0) < sim.scheduler.front_clip().unwrap().end()
    );
    let first_new = sim.jobs.len();
    sim.seek(10.0);
    let actions = sim.step(Some(&track));
    assert!(!actions.iter().any(|a| matches!(
        a,
        FaviconAction::Blank { .. } | FaviconAction::Publish { .. }
    )));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, FaviconAction::SetAllowedFiles(files) if files.is_empty()))
    );
    assert!(
        !sim.allowed
            .contains(&PathBuf::from(old.output.file_name().unwrap()))
    );
    assert!(sim.scheduler.front().is_none());
    assert_eq!(sim.jobs[first_new].start, seconds(10.0));
    assert_eq!(sim.jobs[first_new].duration, seconds(10.0));
    sim.events.push(FaviconEvent::Published(old.output));
    sim.run_until(&track, 24.0);
    sim.assert_covered();
    let mut next = track.clone();
    next.id = "next".into();
    sim.seek(0.0);
    sim.step(Some(&next));
    assert_eq!(sim.jobs.last().unwrap().track_id, "next");
    sim.run_until(&next, 15.0);
    sim.assert_covered();
    assert_eq!(sim.blanks, 0);
    assert_eq!(sim.holds, 3);
    // Looping the same track changes the jump counter even though its identity stays the same.
    sim.seek(0.0);
    sim.step(Some(&next));
    sim.run_until(&next, 20.0);
    sim.assert_covered();
    assert_eq!(sim.holds, 4);
    assert_eq!(sim.blanks, 0);
}

#[test]
fn failed_publish_keeps_old_deck_file_and_does_not_commit_new_clip() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.run_until(&track, 18.0);
    let old_files: Vec<_> = [Deck::A, Deck::B]
        .into_iter()
        .map(|deck| {
            sim.publishes
                .iter()
                .rev()
                .find(|p| p.deck == deck)
                .unwrap()
                .path
                .file_name()
                .unwrap()
                .to_owned()
        })
        .collect();
    let before = sim.publishes.len();
    sim.fail_publish = true;
    let mut failed = None;
    while sim.fail_publish {
        assert!(sim.wall < 70.0);
        sim.advance();
        for action in sim.step(Some(&track)) {
            if let FaviconAction::Publish { path, .. } = action {
                failed = Some(path);
            }
        }
    }
    assert_eq!(sim.publishes.len(), before);
    let failed = failed.unwrap();
    assert!(
        !sim.allowed
            .contains(&PathBuf::from(failed.file_name().unwrap()))
    );
    assert!(
        old_files
            .iter()
            .all(|file| sim.allowed.contains(&PathBuf::from(file)))
    );
    assert_ne!(sim.scheduler.front_clip().unwrap().job.output, failed);
    // The reference polls every 100 ms. An expired front is noticed at the next poll,
    // leaving a bounded gap after this failed handover; preserve and measure that behavior.
    sim.run_until(&track, 39.8);
    let end = sim.scheduler.front_clip().unwrap().end().as_secs_f64();
    sim.media = 39.9;
    sim.wall += 0.1;
    let gap = sim.media - end;
    assert!(gap > 0.0 && gap < 0.1);
    let actions = sim.step(Some(&track));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, FaviconAction::HoldAudio { .. }))
    );
    println!("failed-handover polling gap: {gap:.9} s");
    sim.run_until(&track, 90.0);
    sim.assert_covered();
    assert!(sim.publishes.len() > before);
    assert_eq!(sim.blanks, 0);
}

#[test]
fn failed_first_publish_never_marks_a_clip_live() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.fail_publish = true;
    sim.step(Some(&track));
    while sim.fail_publish {
        assert!(sim.wall < 70.0);
        sim.advance();
        sim.step(Some(&track));
    }
    assert!(sim.scheduler.front().is_none());
    assert!(sim.publishes.is_empty());
    assert!(!sim.music_plays());
}

#[test]
fn seek_replaces_running_encoder_and_ignores_obsolete_completion() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.delay_one = true;
    sim.step(Some(&track));
    let obsolete = sim.jobs[0].clone();
    sim.seek(100.0);
    let actions = sim.step(Some(&track));
    let replace = actions
        .iter()
        .position(|a| matches!(a, FaviconAction::ReplaceEncoder))
        .unwrap();
    let encode = actions
        .iter()
        .position(|a| matches!(a, FaviconAction::Encode(_)))
        .unwrap();
    assert!(replace < encode);
    assert_eq!(sim.replacements, 1);
    sim.events.push(FaviconEvent::Encoded(ReadyClip {
        job: obsolete.clone(),
        frames: 240,
    }));
    sim.run_until(&track, 115.0);
    assert!(!sim.publishes.iter().any(|p| p.path == obsolete.output));
    sim.assert_covered();
    assert!(sim.publishes[0].wall < 6.0);
}

#[test]
fn pause_resume_disable_and_end_blank_only_when_video_stops() {
    let track = track(40);
    let mut sim = Simulation::new();
    sim.run_until(&track, 12.0);
    sim.playing = false;
    sim.step(Some(&track));
    assert_eq!(sim.blanks, 2);
    assert!(sim.step(Some(&track)).is_empty());
    sim.playing = true;
    let first_new = sim.jobs.len();
    sim.step(Some(&track));
    assert_eq!(sim.jobs[first_new].start, seconds(12.0));
    sim.run_until(&track, 20.0);
    sim.assert_covered();
    sim.enabled = false;
    sim.step(Some(&track));
    assert_eq!(sim.blanks, 4);
    sim.enabled = true;
    sim.step(Some(&track));
    sim.run_until(&track, 39.9);
    assert!(
        sim.jobs
            .iter()
            .all(|j| j.start + j.duration <= seconds(40.0))
    );
    sim.playing = false;
    sim.step(Some(&track));
    assert_eq!(sim.blanks, 6);
    let stop = sim.scheduler.stop();
    assert_eq!(
        stop.iter()
            .filter(|a| matches!(a, FaviconAction::Blank { .. }))
            .count(),
        2
    );
}

#[test]
fn late_encode_releases_audio_after_six_seconds_then_rebuilds() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.delay_one = true;
    sim.step(Some(&track));
    let old = sim.jobs[0].clone();
    // This failure simulation deliberately permits uncovered music after the bounded hold expires.
    // Recovery tests below check the deadline and worker replacement, rather than normal coverage.
    while sim.wall < 6.0 {
        sim.advance();
        sim.step(Some(&track));
    }
    assert!(sim.publishes.is_empty());
    assert!(sim.audible_at <= 6.1 + 1e-8);
    let first_new = sim.jobs.len();
    while sim.replacements == 0 {
        assert!(sim.wall < 7.0);
        sim.advance();
        sim.step(Some(&track));
    }
    assert!(sim.wall < 7.0);
    assert_ne!(sim.jobs.last().unwrap().output, old.output);
    assert_eq!(sim.jobs[first_new].duration, seconds(10.0));
}

#[test]
fn gate_can_be_disabled_and_clip_lengths_are_configurable() {
    let mut config = config();
    config.gate = false;
    config.first_clip = seconds(12.0);
    config.clip_seconds = seconds(18.0);
    let track = track(100);
    let mut sim = Simulation::with_config(config);
    sim.step(Some(&track));
    assert_eq!(sim.jobs[0].start, seconds(3.0));
    assert_eq!(sim.jobs[0].duration, seconds(12.0));
    assert_eq!(sim.holds, 0);
    // Gate off intentionally exposes theme video during startup.
    while sim.media < 2.0 {
        sim.advance();
        sim.step(Some(&track));
    }
    assert!(sim.publishes.is_empty());
}

#[test]
fn quiet_can_be_requested_before_encode_completion_but_publish_cannot() {
    let track = track(100);
    let mut config = config();
    config.gate = false;
    let mut sim = Simulation::with_config(config);
    sim.delay_one = true;
    sim.step(Some(&track));
    while sim.wall < 1.8 {
        sim.advance();
        sim.step(Some(&track));
    }
    assert!(!sim.encodes.is_empty());
    assert!(!sim.requests.is_empty());
    assert!(sim.publishes.is_empty());
}

fn snapshot<'a>(
    track: &'a TrackInfo,
    wall: Instant,
    media: f64,
    events: Vec<FaviconEvent>,
) -> FaviconTick<'a> {
    FaviconTick {
        track: Some(track),
        media_time: seconds(media),
        jumps: 0,
        wall_time: wall,
        playing: true,
        enabled: true,
        output_latency: seconds(0.1),
        quiet_granted_at: None,
        events,
    }
}

#[test]
fn hold_ack_supplies_frozen_position_and_stale_ack_is_ignored() {
    let track = track(100);
    let mut scheduler = FaviconScheduler::new(config());
    let wall = Instant::now();
    let actions = scheduler.tick(snapshot(&track, wall, 2.0, vec![])).unwrap();
    let FaviconAction::HoldAudio {
        request_id,
        deadline,
    } = actions[0]
    else {
        panic!("expected hold");
    };
    assert_eq!(deadline, wall + GATE_MAX);
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, FaviconAction::Encode(_)))
    );
    let actions = scheduler
        .tick(snapshot(
            &track,
            wall,
            2.0,
            vec![FaviconEvent::AudioHeld {
                request_id: request_id + 1,
                media_time: seconds(90.0),
            }],
        ))
        .unwrap();
    assert!(actions.is_empty());
    let actions = scheduler
        .tick(snapshot(
            &track,
            wall,
            2.0,
            vec![FaviconEvent::AudioHeld {
                request_id,
                media_time: seconds(2.1),
            }],
        ))
        .unwrap();
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, FaviconAction::Encode(job) if job.start == seconds(2.1)))
    );
}

#[test]
fn missing_hold_ack_releases_at_deadline_without_immediately_holding_again() {
    let track = track(100);
    let mut scheduler = FaviconScheduler::new(config());
    let wall = Instant::now();
    scheduler.tick(snapshot(&track, wall, 0.0, vec![])).unwrap();
    let actions = scheduler
        .tick(snapshot(&track, wall + GATE_MAX, 0.0, vec![]))
        .unwrap();
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, FaviconAction::ReleaseAudio { at: None }))
    );
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, FaviconAction::HoldAudio { .. }))
    );
}

#[test]
fn delayed_publish_ack_uses_completion_time_and_subtracts_output_latency() {
    for latency in [0.0, 0.1, 0.8] {
        let track = track(100);
        let mut scheduler = FaviconScheduler::new(config());
        let wall = Instant::now();
        let actions = scheduler.tick(snapshot(&track, wall, 0.0, vec![])).unwrap();
        let FaviconAction::HoldAudio { request_id, .. } = actions[0] else {
            panic!("expected hold");
        };
        let actions = scheduler
            .tick(snapshot(
                &track,
                wall,
                0.0,
                vec![FaviconEvent::AudioHeld {
                    request_id,
                    media_time: Duration::ZERO,
                }],
            ))
            .unwrap();
        let job = actions
            .into_iter()
            .find_map(|a| {
                if let FaviconAction::Encode(job) = a {
                    Some(job)
                } else {
                    None
                }
            })
            .unwrap();
        let ready = ReadyClip {
            job: job.clone(),
            frames: 240,
        };
        scheduler
            .tick(snapshot(
                &track,
                wall + seconds(2.0),
                0.0,
                vec![FaviconEvent::Encoded(ready)],
            ))
            .unwrap();
        let mut input = snapshot(&track, wall + seconds(3.1), 0.0, vec![]);
        input.quiet_granted_at = Some(wall + seconds(2.6));
        let actions = scheduler.tick(input).unwrap();
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, FaviconAction::Publish { path, .. } if *path == job.output))
        );
        assert!(scheduler.front().is_none());
        assert!(!actions.iter().any(|a| matches!(
            a,
            FaviconAction::SetLive(true) | FaviconAction::ReleaseAudio { .. }
        )));
        let mut input = snapshot(
            &track,
            wall + seconds(3.8),
            0.0,
            vec![FaviconEvent::PublishedAt {
                path: job.output,
                wall_time: wall + seconds(3.2),
            }],
        );
        input.output_latency = seconds(latency);
        let actions = scheduler.tick(input).unwrap();
        let expected = wall + seconds(3.2) + seconds(0.5).saturating_sub(seconds(latency));
        assert!(
            actions.iter().any(
                |a| matches!(a, FaviconAction::ReleaseAudio { at: Some(at) } if *at == expected)
            )
        );
        assert_eq!(scheduler.front(), Some(Deck::A));
    }
}

#[test]
fn newest_deck_plan_overlaps_and_reaches_the_track_end() {
    let track = track(241);
    let mut sim = Simulation::new();
    sim.run_until(&track, 240.8);
    assert!(sim.scheduler.front_clip().unwrap().end() >= seconds(241.0));
    for pair in sim.jobs.windows(2) {
        assert_ne!(pair[0].deck, pair[1].deck);
        let overlap =
            (pair[0].start + pair[0].duration).as_secs_f64() - pair[1].start.as_secs_f64();
        assert!((overlap - 3.0).abs() < 0.05, "handover {overlap}");
    }
    sim.media = 241.03; // The PCM tail extends beyond the final complete video frame.
    let actions = sim.step(Some(&track));
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, FaviconAction::HoldAudio { .. }))
    );
    assert!(sim.scheduler.front().is_some());
    sim.seek(0.0);
    let actions = sim.step(Some(&track));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, FaviconAction::HoldAudio { .. }))
    );
}

#[test]
fn queued_or_finished_encodes_do_not_replace_worker_on_seek() {
    for completed in [false, true] {
        let track = track(100);
        let mut config = config();
        config.gate = false;
        let mut scheduler = FaviconScheduler::new(config);
        let wall = Instant::now();
        let job = scheduler
            .tick(snapshot(&track, wall, 0.0, vec![]))
            .unwrap()
            .into_iter()
            .find_map(|a| {
                if let FaviconAction::Encode(job) = a {
                    Some(job)
                } else {
                    None
                }
            })
            .unwrap();
        let events = if completed {
            vec![
                FaviconEvent::EncodeStarted(job.clone()),
                FaviconEvent::Encoded(ReadyClip { job, frames: 240 }),
            ]
        } else {
            vec![]
        };
        let mut input = snapshot(&track, wall, 20.0, events);
        input.jumps = 1;
        let actions = scheduler.tick(input).unwrap();
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, FaviconAction::CancelPending))
        );
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, FaviconAction::ReplaceEncoder))
        );
    }
}

#[test]
fn missing_quiet_grant_prevents_publication_and_bounds_the_plan() {
    let track = track(100);
    let mut sim = Simulation::new();
    sim.grant_quiet = false;
    while sim.wall < 5.9 {
        sim.step(Some(&track));
        sim.advance();
    }
    assert!(sim.publishes.is_empty());
    assert!(sim.scheduler.front().is_none());
    assert_eq!(sim.jobs.len(), 2);
    assert_eq!(sim.requests.len(), 1);
}

#[test]
fn encode_failure_is_consumed_and_late_success_cannot_revive_it() {
    let track = track(100);
    let mut sim = Simulation::new();
    sim.step(Some(&track));
    let failed = sim.jobs[0].clone();
    sim.encodes.clear();
    sim.worker_free = sim.wall;
    sim.events.push(FaviconEvent::EncodeFailed(failed.clone()));
    sim.step(Some(&track));
    sim.events.push(FaviconEvent::Encoded(ReadyClip {
        job: failed.clone(),
        frames: 240,
    }));
    while sim.wall < 3.0 {
        sim.advance();
        sim.step(Some(&track));
    }
    assert!(!sim.publishes.iter().any(|p| p.path == failed.output));
    assert!(
        !sim.allowed
            .contains(&PathBuf::from(failed.output.file_name().unwrap()))
    );
}

#[test]
fn a_dropped_late_plan_pushes_the_next_start_later() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.run_until(&track, 10.0);
    let work = sim.encodes.iter_mut().find(|w| w.started).unwrap();
    work.due += 40.0;
    sim.worker_free += 40.0;
    while sim.replacements == 0 {
        assert!(sim.wall < 70.0, "late work must be replaced");
        sim.advance();
        sim.step(Some(&track));
    }
    assert_eq!(sim.blanks, 0);
    let replacement = sim.jobs.last().unwrap();
    assert!(replacement.start.as_secs_f64() > sim.media);
    assert!(replacement.start.as_secs_f64() > 36.839160839);
}
