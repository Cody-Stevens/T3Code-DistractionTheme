//! Fake-clock replay of the measured Python worker, loading, long-write and handover timings.
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

fn scheduler() -> FaviconScheduler {
    FaviconScheduler::new(FaviconConfig {
        clips_dir: std::env::temp_dir().join("fake-clips-no-io"),
        project_id: "project-a".into(),
        deck_b_project_id: Some("project-b".into()),
        run_id: "fake-run".into(),
        show_latency: seconds(0.5),
    })
}

struct Simulation {
    scheduler: FaviconScheduler,
    epoch: Instant,
    wall: f64,
    media: f64,
    worker_free: f64,
    encodes: Vec<(f64, ClipJob)>,
    events: Vec<FaviconEvent>,
    grant_due: Option<f64>,
    grant: Option<Instant>,
    jobs: Vec<ClipJob>,
    publishes: Vec<(f64, Deck, PathBuf)>,
    requests: Vec<f64>,
    flips: Vec<(f64, Deck)>,
    playing: bool,
    enabled: bool,
    delay_one: bool,
    quiet_build: f64,
}

impl Simulation {
    fn new() -> Self {
        Self {
            scheduler: scheduler(),
            epoch: Instant::now(),
            wall: 0.0,
            media: 0.0,
            worker_free: 0.0,
            encodes: Vec::new(),
            events: Vec::new(),
            grant_due: None,
            grant: None,
            jobs: Vec::new(),
            publishes: Vec::new(),
            requests: Vec::new(),
            flips: Vec::new(),
            playing: true,
            enabled: true,
            delay_one: false,
            quiet_build: 0.6,
        }
    }

    fn step(&mut self, track: Option<&TrackInfo>) -> Vec<FaviconAction> {
        if self.grant_due.is_some_and(|due| self.wall + 1e-8 >= due) {
            self.grant = Some(self.epoch + seconds(self.grant_due.take().unwrap()));
        }
        let mut future = Vec::new();
        for (due, job) in self.encodes.drain(..) {
            if due <= self.wall + 1e-8 {
                let frames = (job.duration.as_secs_f64() * 24.0).round() as u64;
                self.events
                    .push(FaviconEvent::Encoded(ReadyClip { job, frames }));
            } else {
                future.push((due, job));
            }
        }
        self.encodes = future;
        let actions = self
            .scheduler
            .tick(FaviconTick {
                track,
                media_time: seconds(self.media),
                wall_time: self.epoch + seconds(self.wall),
                playing: self.playing,
                enabled: self.enabled,
                quiet_granted_at: self.grant,
                events: std::mem::take(&mut self.events),
            })
            .unwrap();
        for action in &actions {
            match action {
                FaviconAction::Encode(job) => {
                    // Measured costs without the planner's 1.3 safety factor; one serial worker.
                    let cost = 0.3
                        + job.duration.as_secs_f64() * if job.speed == 10 { 0.15 } else { 0.22 };
                    let mut due = self.wall.max(self.worker_free) + cost;
                    if self.delay_one {
                        due += 20.0;
                        self.delay_one = false;
                    }
                    self.worker_free = due;
                    self.encodes.push((due, job.clone()));
                    self.jobs.push(job.clone());
                }
                FaviconAction::RequestQuiet => {
                    assert!(self.grant_due.is_none());
                    self.grant_due = Some(self.wall + self.quiet_build);
                    self.requests.push(self.media);
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
                    self.publishes.push((self.media, *deck, path.clone()));
                    self.events.push(FaviconEvent::Published(path.clone()));
                }
                FaviconAction::SetFront(Some(deck)) => self.flips.push((self.media, *deck)),
                FaviconAction::CancelPending => {
                    self.encodes.clear();
                    self.worker_free = self.wall;
                }
                FaviconAction::Cleanup { keep, .. } => assert_eq!(*keep, 30),
                _ => (),
            }
        }
        actions
    }

    fn advance(&mut self) {
        // Integer-grid clocks avoid cumulative floating-point error at the 600 ms cutoff.
        self.wall = ((self.wall * 10.0).round() + 1.0) / 10.0;
        if self.playing {
            self.media = ((self.media * 10.0).round() + 1.0) / 10.0;
        }
    }

    fn run_until(&mut self, track: &TrackInfo, media: f64) {
        while self.media < media {
            self.step(Some(track));
            self.advance();
        }
    }

    fn assert_covered(&self) {
        let clip = self
            .scheduler
            .front_clip()
            .expect("steady playback must have a front clip");
        assert!(
            clip.job.start <= seconds(self.media) && seconds(self.media) < clip.end(),
            "front covers {}..{}, media {}",
            clip.job.start.as_secs_f64(),
            clip.end().as_secs_f64(),
            self.media
        );
        let publish = self
            .publishes
            .iter()
            .find(|(_, _, path)| path == &clip.job.output)
            .unwrap()
            .0;
        assert!(self.media + 1e-8 >= publish + 0.5, "front must have loaded");
    }
}

#[test]
fn cold_start_and_steady_ramp_have_no_front_coverage_gaps() {
    let track = track(400);
    let mut sim = Simulation::new();
    let first = sim.step(Some(&track));
    assert!(
        matches!(&first[0], FaviconAction::Encode(job) if job.start == seconds(3.0) && job.duration == seconds(6.0) && job.deck == Deck::A)
    );
    assert!(sim.scheduler.front().is_none());
    let mut samples = 0;
    while sim.media < 300.0 {
        sim.advance();
        sim.step(Some(&track));
        if sim.media >= 3.0 {
            sim.assert_covered();
            samples += 1;
        }
    }
    assert_eq!(samples, 2971);
    let lengths: Vec<_> = sim
        .jobs
        .iter()
        .take(4)
        .map(|job| job.duration.as_secs_f64())
        .collect();
    assert!((lengths[0] - 6.0).abs() < 0.01);
    assert!((lengths[1] - 12.167832168).abs() < 1e-6, "{lengths:?}");
    assert!((lengths[2] - 31.006406182).abs() < 1e-6, "{lengths:?}");
    assert_eq!(lengths[3], 48.0); // 45 seconds plus 3 seconds handover
    // Golden values from reference_replay.py, which extracts the actual Python methods.
    for (job, expected) in sim.jobs.iter().zip([3.0, 6.6, 15.767832168, 43.774238349]) {
        assert!((job.start.as_secs_f64() - expected).abs() < 1e-6);
    }
    for ((at, _, _), expected) in sim.publishes.iter().zip([2.4, 6.0, 15.2, 43.2]) {
        assert_eq!(*at, expected);
    }
    for ((at, _), expected) in sim.flips.iter().zip([2.4, 7.2, 16.4, 44.4]) {
        assert_eq!(*at, expected);
    }
    // Prove continuous interval coverage between flips, in addition to sampling at every tick.
    // Within an interval, media advances continuously and the selected clip never changes.
    for (index, (begin, deck)) in sim.flips.iter().enumerate() {
        let end = sim.flips.get(index + 1).map_or(300.0, |(at, _)| *at);
        let (published, _, output) = sim
            .publishes
            .iter()
            .rev()
            .find(|(at, d, _)| d == deck && at <= begin)
            .unwrap();
        let job = sim.jobs.iter().find(|job| job.output == *output).unwrap();
        let clip_end = job.start.as_secs_f64() + (job.duration.as_secs_f64() * 24.0).round() / 24.0;
        let covered_from = begin.max(3.0);
        assert!(job.start.as_secs_f64() <= covered_from);
        assert!(published + 0.5 <= covered_from + 1e-8);
        assert!(
            clip_end >= end,
            "gap before flip at {end}, clip ends at {clip_end}"
        );
    }
    assert!(sim.flips.len() >= 8);
    for pair in sim.publishes.windows(2) {
        assert_ne!(pair[0].1, pair[1].1);
    }
    for (published, _, path) in &sim.publishes {
        let job = sim.jobs.iter().find(|job| job.output == *path).unwrap();
        let offset = published + 0.5 - job.start.as_secs_f64();
        assert!(offset.abs() <= 0.2, "publication A/V offset {offset}");
    }
    let unique: HashSet<_> = sim.jobs.iter().map(|job| &job.output).collect();
    assert_eq!(unique.len(), sim.jobs.len());
    assert!((sim.requests[0] - 1.3).abs() < 0.01);
    for (at, deck) in sim.flips.iter().skip(1) {
        let publish = sim
            .publishes
            .iter()
            .rev()
            .find(|(publish, d, _)| d == deck && publish <= at)
            .unwrap();
        assert!(*at - publish.0 >= 1.2 - 1e-8);
    }
    println!(
        "steady coverage: {samples}/{samples} samples from 3.0 through 300.0 s; ramp {lengths:?}"
    );
}

#[test]
fn seek_and_track_change_blank_both_decks_and_ignore_stale_completions() {
    for target in [0.0, 150.0] {
        let track = track(400);
        let mut sim = Simulation::new();
        sim.run_until(&track, 20.0);
        let obsolete = sim.jobs.last().unwrap().clone();
        sim.media = target;
        sim.events.push(FaviconEvent::Encoded(ReadyClip {
            job: obsolete.clone(),
            frames: 1000,
        }));
        let actions = sim.step(Some(&track));
        assert_eq!(
            actions
                .iter()
                .filter(|a| matches!(a, FaviconAction::Blank { .. }))
                .count(),
            2
        );
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, FaviconAction::CancelPending))
        );
        assert!(sim.scheduler.front().is_none());
        let newest = sim.jobs.last().unwrap();
        assert_eq!(newest.start, seconds(target + 3.0));
        assert_eq!(newest.duration, seconds(6.0));
        assert_ne!(newest.output, obsolete.output);
        let end = target + 12.0;
        while sim.media < end {
            sim.advance();
            sim.step(Some(&track));
            if sim.media >= target + 3.0 {
                sim.assert_covered();
            }
        }
        let mut next_track = track.clone();
        next_track.id = "next".into();
        sim.media = 0.0;
        let actions = sim.step(Some(&next_track));
        assert_eq!(
            actions
                .iter()
                .filter(|a| matches!(a, FaviconAction::Blank { .. }))
                .count(),
            2
        );
        assert_eq!(sim.jobs.last().unwrap().track_id, "next");
    }
}

#[test]
fn track_end_stops_planning_and_blanks_both_decks() {
    let track = track(40);
    let mut sim = Simulation::new();
    sim.run_until(&track, 40.0);
    for job in &sim.jobs {
        assert!(job.start + job.duration <= seconds(40.0));
    }
    sim.playing = false; // The player stops when PCM reaches its final sample.
    let actions = sim.step(Some(&track));
    assert!(sim.scheduler.front().is_none());
    assert_eq!(
        actions
            .iter()
            .filter(|a| matches!(a, FaviconAction::Blank { .. }))
            .count(),
        2
    );
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, FaviconAction::Encode(_)))
    );
    sim.advance();
    assert!(sim.step(None).is_empty());
}

#[test]
fn late_encode_drops_the_plan_and_restarts_with_a_unique_clip() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.delay_one = true;
    sim.run_until(&track, 3.7);
    let first = sim.jobs[0].clone();
    let actions = sim.step(Some(&track));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, FaviconAction::CancelPending))
    );
    let restarted = sim.jobs.last().unwrap().clone();
    assert_eq!(restarted.start, seconds(6.7));
    assert_eq!(restarted.duration, seconds(6.0));
    sim.events.push(FaviconEvent::Encoded(ReadyClip {
        job: first.clone(),
        frames: 144,
    }));
    sim.advance();
    sim.step(Some(&track));
    sim.run_until(&track, 10.0);
    assert!(
        !sim.publishes
            .iter()
            .any(|(_, _, path)| path == &first.output)
    );
    assert!(
        sim.publishes
            .iter()
            .any(|(_, _, path)| path == &restarted.output)
    );
    sim.assert_covered();
}

#[test]
fn pause_disable_and_stop_cancel_and_blank_without_republishing() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.run_until(&track, 12.0);
    sim.playing = false;
    let actions = sim.step(Some(&track));
    assert_eq!(
        actions
            .iter()
            .filter(|a| matches!(a, FaviconAction::Blank { .. }))
            .count(),
        2
    );
    assert!(sim.scheduler.front().is_none());
    sim.advance();
    assert!(sim.step(Some(&track)).is_empty());
    sim.playing = true;
    sim.step(Some(&track));
    sim.enabled = false;
    let actions = sim.step(Some(&track));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, FaviconAction::CancelPending))
    );
    assert_eq!(
        sim.scheduler
            .stop()
            .iter()
            .filter(|a| matches!(a, FaviconAction::Blank { .. }))
            .count(),
        2
    );
}

#[test]
fn no_quiet_grant_means_no_publication_and_failed_api_stays_optimistically_live() {
    let track = track(400);
    let mut sim = Simulation::new();
    while sim.media < 2.5 {
        sim.step(Some(&track));
        sim.grant_due = None;
        sim.advance();
    }
    assert!(sim.publishes.is_empty());
    // A long write lands at 2.5: publication cannot start before 2.95.
    sim.grant = Some(sim.epoch + seconds(2.5));
    sim.step(Some(&track));
    assert!(sim.publishes.is_empty());
    sim.run_until(&track, 3.1);
    let path = sim.publishes.last().unwrap().2.clone();
    sim.events.push(FaviconEvent::PublishFailed(path));
    sim.step(Some(&track));
    // Deliberate Python compatibility: an API failure is swallowed and live remains true.
    assert_eq!(sim.scheduler.front(), Some(Deck::A));
}

#[test]
fn slow_cold_long_write_still_publishes_before_the_late_cutoff() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.quiet_build = 1.6; // Python's measured first-write cost after cold start/seek.
    sim.run_until(&track, 3.5);
    sim.step(Some(&track));
    assert_eq!(sim.publishes[0].0, 3.4);
    assert_eq!(sim.scheduler.front(), Some(Deck::A));
}

#[test]
fn encode_failure_is_consumed_at_publish_time_and_old_completion_cannot_revive_it() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.step(Some(&track));
    let failed = sim.jobs[0].clone();
    sim.encodes.clear();
    sim.events.push(FaviconEvent::EncodeFailed(failed.clone()));
    sim.run_until(&track, 2.5);
    sim.events.push(FaviconEvent::Encoded(ReadyClip {
        job: failed.clone(),
        frames: 144,
    }));
    sim.step(Some(&track));
    assert!(
        !sim.publishes
            .iter()
            .any(|(_, _, output)| output == &failed.output)
    );
}

#[test]
fn in_range_seek_preserves_pythons_undetected_seek_behavior() {
    let track = track(400);
    let mut sim = Simulation::new();
    sim.run_until(&track, 12.0);
    let old = sim.scheduler.front_clip().unwrap().job.output.clone();
    sim.media = 10.0; // Still inside the current clip; Python detects seeks only by range.
    let actions = sim.step(Some(&track));
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, FaviconAction::Blank { .. }))
    );
    assert_eq!(sim.scheduler.front_clip().unwrap().job.output, old);
}
