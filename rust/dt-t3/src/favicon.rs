//! Pure port of `glowtube.FaviconVideo`. The binary executes effects and feeds back completions.
//! No tick sleeps, encodes, touches files or calls HTTP. Media time and wall time stay separate.

use dt_core::{Error, Result, TrackInfo, constants::FPS};
use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, Instant},
};

/// Number of old clips retained for clients that fetch a favicon late.
pub const RETAIN_CLIPS: usize = 30;
/// Tiny inert favicon the binary writes to clips_dir/glowtube-blank.svg before executing blanks.
pub const BLANK_SVG: &str =
    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"/>";
/// Long theme-write content duration requested before each publication.
pub const QUIET_CONTENT: Duration = Duration::from_millis(2500);
/// Period in which the binary must suppress subsequent theme writes.
pub const QUIET_HOLD: Duration = Duration::from_millis(2200);
/// Debounce/paint settling time after the long theme write lands.
pub const QUIET_SETTLE: Duration = Duration::from_millis(450);
/// Hard limit for a first-clip audio hold, also enforced by the binary.
pub const GATE_MAX: Duration = Duration::from_secs(6);
const FLIP_AFTER: Duration = Duration::from_millis(1200);
const QUIET_BUILD: f64 = 0.8;
const ENCODE_START: f64 = 0.3;
const FAST_COST: f64 = 0.15 * 1.3;
const GOOD_COST: f64 = 0.22 * 1.3;

/// Favicon carrier project, also embedded in clip filenames for the renderer's front-deck rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deck {
    /// GlowTube, rooted at the helper workspace.
    A,
    /// GlowTube B, rooted at deck-b.
    B,
}

impl Deck {
    /// Filename component and argument to `favicon_front_css`.
    pub fn name(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }
}

/// Paths, project identities and measured show latency. Construction has no side effects.
#[derive(Clone, Debug)]
pub struct FaviconConfig {
    /// Absolute clip directory. The binary limits cleanup to owned glowtube-clip-*.svg files here.
    pub clips_dir: PathBuf,
    /// GlowTube project ID (deck A).
    pub project_id: String,
    /// GlowTube B ID. None preserves Python's single-deck fallback with visible swap gaps.
    pub deck_b_project_id: Option<String>,
    /// Unique run component, normally a UUID; every queued clip also gets a sequence number.
    pub run_id: String,
    /// API-to-first-frame latency, normally 500 ms.
    pub show_latency: Duration,
    /// Optional first-clip gate latency for encoders whose short fast clips load sooner.
    /// None uses show_latency, preserving Python's common latency for all clips.
    pub first_show_latency: Option<Duration>,
    /// Equivalent to GLOWTUBE_FIRST_CLIP; normally 10 seconds, encoded fast.
    pub first_clip: Duration,
    /// Equivalent to GLOWTUBE_CLIP_SECONDS; normally 22 seconds, plus handover.
    pub clip_seconds: Duration,
    /// Equivalent to GLOWTUBE_GATE != 0; hold music until the first picture is ready.
    pub gate: bool,
}

/// Encoder request, adapted by the binary to dt-render's SVG encoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipJob {
    /// Identity used to reject obsolete completions.
    pub track_id: String,
    /// Source media file.
    pub video: PathBuf,
    /// Target deck; filenames contain the same a/b component.
    pub deck: Deck,
    /// First frame's media time.
    pub start: Duration,
    /// Full duration, including the overlap (normally up to 25 seconds).
    pub duration: Duration,
    /// Unique absolute SVG destination.
    pub output: PathBuf,
    /// Python speed intent: 10 for fast startup/ramp, 8 for longer good-quality clips.
    pub speed: u8,
    /// Python quality intent: 55 for fast, 60 for good.
    pub quality: u8,
    /// Estimated serial-worker completion on the media clock, including the 1.3 cost margin.
    pub done_by: Duration,
}

/// Encoded result. Actual frame count, rather than the planned duration, determines media end.
#[derive(Clone, Debug)]
pub struct ReadyClip {
    /// Original request; exact job matching rejects cancelled/obsolete work.
    pub job: ClipJob,
    /// Actual number of 24 fps frames.
    pub frames: u64,
}

impl ReadyClip {
    /// Exclusive media end of the encoded clip.
    pub fn end(&self) -> Duration {
        self.job
            .start
            .saturating_add(Duration::from_secs_f64(self.frames as f64 / f64::from(FPS)))
    }
}

/// Worker/API completions delivered by orchestration.
#[derive(Clone, Debug)]
pub enum FaviconEvent {
    /// The worker actually started this job. Deliver before any tick that might cancel it.
    EncodeStarted(ClipJob),
    /// Encoding completed.
    Encoded(ReadyClip),
    /// Encoding failed. Python consumes the failed result when publication becomes due.
    EncodeFailed(ClipJob),
    /// Publication succeeded. Only this acknowledgment commits the deck's new clip/file.
    Published(PathBuf),
    /// Publication completed on the API worker at this wall time, before a later owner tick.
    PublishedAt {
        /// Successfully published clip path.
        path: PathBuf,
        /// Actual API completion timestamp; do not replace it with event-consumption time.
        wall_time: Instant,
    },
    /// Publication failed; the deck keeps its previous clip/file.
    PublishFailed(PathBuf),
    /// Audio is silent and its clock frozen; encode the first clip at this exact position.
    AudioHeld {
        /// Echo the HoldAudio request; obsolete acknowledgments are ignored.
        request_id: u64,
        /// Frozen player position, which may differ from the preceding audible snapshot.
        media_time: Duration,
    },
}

/// Player snapshot. Call about every 100 ms and deliver completions exactly once.
pub struct FaviconTick<'a> {
    /// Loaded track; Some implies PCM is ready. A missing source path disables favicon video.
    pub track: Option<&'a TrackInfo>,
    /// Audible media clock, frozen while paused and discontinuous after seeks.
    pub media_time: Duration,
    /// Player's explicit seek/track-change counter, including jumps within the current clip.
    pub jumps: u64,
    /// Injected monotonic wall time; tests advance an Instant without sleeping.
    pub wall_time: Instant,
    /// False invalidates outstanding jobs and blanks both decks.
    pub playing: bool,
    /// False for favicon off.
    pub enabled: bool,
    /// Audio output buffering latency, subtracted from the first-picture release delay.
    pub output_latency: Duration,
    /// Time the long theme write landed. Scheduler adds QUIET_SETTLE before allowing a publish.
    /// Retain the latest grant in later snapshots; old grants cannot authorize a new request.
    pub quiet_granted_at: Option<Instant>,
    /// Completions since the previous tick; stale jobs are ignored.
    pub events: Vec<FaviconEvent>,
}

/// Effects only. Execute in order on the binary's worker, outside the audio callback.
#[derive(Clone, Debug)]
pub enum FaviconAction {
    /// Queue on the dedicated serial favicon worker, separate from the six short-clip workers.
    Encode(ClipJob),
    /// Cancel all outstanding favicon jobs where possible; obsolete completions remain harmless.
    CancelPending,
    /// Terminate the busy favicon worker (including its decoder), create and warm a replacement.
    /// Execute after CancelPending and before any subsequent Encode.
    ReplaceEncoder,
    /// Silence audio and freeze its clock, then return AudioHeld with the frozen position.
    HoldAudio {
        /// Identity echoed by AudioHeld.
        request_id: u64,
        /// Release even if the scheduler or encoder stops responding.
        deadline: Instant,
    },
    /// Release immediately (None) or at a monotonic deadline, outside the scheduler tick.
    ReleaseAudio {
        /// Successful publication uses show latency minus audio output latency.
        at: Option<Instant>,
    },
    /// Build QUIET_CONTENT of cheap theme video, land it, then hold writes for QUIET_HOLD.
    RequestQuiet,
    /// Set faviconPath; feed Published/PublishFailed back on a later tick.
    Publish {
        /// Deck receiving the clip, normally the back deck.
        deck: Deck,
        /// Destination project ID.
        project_id: String,
        /// Encoded SVG path.
        path: PathBuf,
    },
    /// Publish the tiny blank SVG on this deck. The binary creates it; clips remain available.
    Blank {
        /// Deck being blanked.
        deck: Deck,
        /// Destination project ID.
        project_id: String,
        /// clips_dir/glowtube-blank.svg.
        path: PathBuf,
    },
    /// Apply this deck's front rule in the next complete theme write, or remove it for None.
    SetFront(Option<Deck>),
    /// Filenames allowed by favicon_allow_css: current deck files and planned/publishing files.
    /// Apply in every complete theme write; an empty list hides every stale clip.
    SetAllowedFiles(Vec<PathBuf>),
    /// Delete only old owned clips, retaining the newest `keep` files for late fetches.
    Cleanup {
        /// Directory from the injected configuration.
        clips_dir: PathBuf,
        /// Retention count, normally 30.
        keep: usize,
    },
    /// Cheap hidden theme-video encoding is appropriate while a front clip is live.
    SetLive(bool),
}

enum Completion {
    Waiting,
    Ready(ReadyClip),
    Failed,
}

struct Planned {
    job: ClipJob,
    completion: Completion,
    running: bool,
}

struct Gate {
    request_id: u64,
    deadline: Instant,
    position: Option<Duration>,
    release_at: Option<Instant>,
}

#[derive(Default)]
struct DeckState {
    clip: Option<ReadyClip>,
    published_at: Option<Instant>,
    blank: bool,
}

/// Python's clip planning and quiet-window state, with all effects delegated to the binary.
pub struct FaviconScheduler {
    config: FaviconConfig,
    decks: [DeckState; 2],
    front: Option<Deck>,
    plan: VecDeque<Planned>,
    publishing: Option<ReadyClip>,
    gate: Option<Gate>,
    gate_sequence: u64,
    sequence: u64,
    jumps_seen: u64,
    quiet_requested_at: Option<Instant>,
    latest_grant: Option<Instant>,
    last_wall: Option<Instant>,
}

impl FaviconScheduler {
    /// Initialize without HTTP/filesystem side effects; new runs should use unique run IDs.
    pub fn new(config: FaviconConfig) -> Self {
        Self {
            config,
            decks: Default::default(),
            front: None,
            plan: VecDeque::new(),
            publishing: None,
            gate: None,
            gate_sequence: 0,
            sequence: 0,
            jumps_seen: 0,
            quiet_requested_at: None,
            latest_grant: None,
            last_wall: None,
        }
    }

    /// Current CSS front deck; publication must succeed before a deck becomes live.
    pub fn front(&self) -> Option<Deck> {
        self.front
    }

    /// Clip the scheduler believes is on top; use end() and show latency to assess coverage.
    pub fn front_clip(&self) -> Option<&ReadyClip> {
        self.front
            .and_then(|deck| self.decks[deck.index()].clip.as_ref())
    }

    /// Advance the adaptive plan without performing I/O. First clips are gated, 10-second fast
    /// encodes; later clips normally stop at 22 seconds plus handover. Late cutoff is 600 ms.
    pub fn tick(&mut self, input: FaviconTick<'_>) -> Result<Vec<FaviconAction>> {
        if self.last_wall.is_some_and(|last| input.wall_time < last) {
            return Err(Error::InvalidInput("favicon wall clock moved backwards"));
        }
        self.last_wall = Some(input.wall_time);
        let mut actions = Vec::new();
        if let Some(grant) = input.quiet_granted_at
            && self.latest_grant.is_none_or(|last| grant > last)
            && grant <= input.wall_time
        {
            self.latest_grant = Some(grant);
            if self
                .quiet_requested_at
                .is_some_and(|request| grant >= request)
            {
                self.quiet_requested_at = None;
            }
        }
        // Record worker completions before cancellation: a finished job needs no replacement.
        for event in &input.events {
            match event {
                FaviconEvent::EncodeStarted(job) => {
                    if let Some(planned) = self.plan.iter_mut().find(|p| p.job == *job)
                        && matches!(planned.completion, Completion::Waiting)
                    {
                        planned.running = true;
                    }
                }
                FaviconEvent::Encoded(clip) => {
                    if let Some(planned) = self.plan.iter_mut().find(|p| p.job == clip.job)
                        && matches!(planned.completion, Completion::Waiting)
                    {
                        planned.completion = Completion::Ready(clip.clone());
                        planned.running = false;
                    }
                }
                FaviconEvent::EncodeFailed(job) => {
                    if let Some(planned) = self.plan.iter_mut().find(|p| p.job == *job)
                        && matches!(planned.completion, Completion::Waiting)
                    {
                        planned.completion = Completion::Failed;
                        planned.running = false;
                    }
                }
                _ => (),
            }
        }
        let track = input
            .track
            .filter(|track| track.path.is_some() && input.playing && input.enabled);
        let Some(track) = track else {
            self.blank_all(&mut actions, false);
            return Ok(actions);
        };
        let gate_expired = self
            .gate
            .as_ref()
            .is_some_and(|gate| input.wall_time >= gate.deadline && gate.release_at.is_none());
        if let Some(gate) = &self.gate {
            if gate_expired {
                self.release_audio(&mut actions);
            } else if gate.release_at.is_some_and(|at| input.wall_time >= at) {
                self.gate = None; // The binary already scheduled the release.
            }
        }
        let mut now = input.media_time.as_secs_f64();
        let jumped = input.jumps != self.jumps_seen;
        self.jumps_seen = input.jumps;
        if ((self.front.is_some()
            || !self.plan.is_empty()
            || self.publishing.is_some()
            || self.gate.is_some())
            && jumped)
            || self.front_clip().is_some_and(|clip| {
                clip.job.track_id != track.id
                    || now < clip.job.start.as_secs_f64() - 0.6
                    // PCM can outlast the last complete video frame by a fraction of a frame.
                    // Keep the terminal picture until the player advances the track; restarting
                    // here would hold the clock with less than one second left to encode.
                    || (input.media_time >= clip.end()
                        && clip.end().as_secs_f64()
                            < track.frames as f64 / f64::from(FPS) - 0.5)
            })
        {
            self.restart(&mut actions);
        }
        if self
            .plan
            .front()
            .is_some_and(|p| p.job.track_id != track.id || now > p.job.start.as_secs_f64() + 0.6)
        {
            self.drop_plan(&mut actions);
        }
        // Reset first, so an acknowledgment racing a seek cannot revive an obsolete publication.
        for event in input.events {
            match event {
                FaviconEvent::AudioHeld {
                    request_id,
                    media_time,
                } => {
                    if let Some(gate) = &mut self.gate
                        && gate.request_id == request_id
                        && gate.position.is_none()
                    {
                        gate.position = Some(media_time);
                        now = media_time.as_secs_f64();
                    }
                }
                FaviconEvent::Published(_) | FaviconEvent::PublishedAt { .. } => {
                    let (path, published_at) = match event {
                        FaviconEvent::Published(path) => (path, input.wall_time),
                        FaviconEvent::PublishedAt { path, wall_time } => {
                            (path, wall_time.min(input.wall_time))
                        }
                        _ => unreachable!(),
                    };
                    if self
                        .publishing
                        .as_ref()
                        .is_some_and(|clip| clip.job.output == path)
                    {
                        let clip = self.publishing.take().expect("matched publication");
                        let deck = clip.job.deck;
                        if let Some(gate) = &mut self.gate
                            && gate.release_at.is_none()
                        {
                            let at = (published_at
                                + self
                                    .config
                                    .first_show_latency
                                    .unwrap_or(self.config.show_latency)
                                    .saturating_sub(input.output_latency))
                            .min(gate.deadline);
                            gate.release_at = Some(at);
                            actions.push(FaviconAction::ReleaseAudio { at: Some(at) });
                        }
                        self.decks[deck.index()] = DeckState {
                            clip: Some(clip),
                            published_at: Some(published_at),
                            blank: false,
                        };
                        self.allow(&mut actions);
                        if self.front.is_none() || !self.dual() {
                            self.set_front(Some(deck), &mut actions);
                        }
                        actions.push(FaviconAction::Cleanup {
                            clips_dir: self.config.clips_dir.clone(),
                            keep: RETAIN_CLIPS,
                        });
                    }
                }
                FaviconEvent::PublishFailed(path)
                    if self
                        .publishing
                        .as_ref()
                        .is_some_and(|clip| clip.job.output == path) =>
                {
                    self.publishing = None;
                    self.allow(&mut actions);
                }
                _ => (),
            }
        }
        // Match Python's synchronous publication: do not schedule another mutation while in flight.
        if self.publishing.is_some() {
            return Ok(actions);
        }
        for deck in self.active_decks() {
            let state = &self.decks[deck.index()];
            if Some(deck) != self.front
                && state.clip.as_ref().is_some_and(|clip| {
                    clip.job.track_id == track.id
                        && clip.job.start.as_secs_f64() <= now
                        && now < clip.end().as_secs_f64()
                        && self
                            .front_clip()
                            .is_none_or(|front| clip.job.start > front.job.start)
                })
                && state
                    .published_at
                    .is_some_and(|at| input.wall_time.saturating_duration_since(at) >= FLIP_AFTER)
            {
                self.set_front(Some(deck), &mut actions);
                break;
            }
        }
        self.plan_next(track, now, input.wall_time, !gate_expired, &mut actions);
        let Some(next) = self.plan.front() else {
            return Ok(actions);
        };
        if self.dual() && Some(next.job.deck) == self.front {
            return Ok(actions);
        }
        let done = !matches!(next.completion, Completion::Waiting);
        let start = next.job.start.as_secs_f64();
        let grant_settled = self
            .latest_grant
            .and_then(|at| at.checked_add(QUIET_SETTLE));
        let can_request = self.quiet_requested_at.is_none()
            && grant_settled.is_none_or(|at| {
                input
                    .wall_time
                    .checked_duration_since(at)
                    .is_some_and(|elapsed| elapsed > QUIET_HOLD)
            });
        if (done || now >= next.job.done_by.as_secs_f64() - 0.5)
            && now
                >= start
                    - self.config.show_latency.as_secs_f64()
                    - QUIET_SETTLE.as_secs_f64()
                    - QUIET_BUILD
            && can_request
        {
            self.quiet_requested_at = Some(input.wall_time);
            actions.push(FaviconAction::RequestQuiet);
        } else if done
            && now >= start - self.config.show_latency.as_secs_f64() - 0.1
            && grant_settled.is_some_and(|at| {
                input
                    .wall_time
                    .checked_duration_since(at)
                    .is_some_and(|elapsed| elapsed < QUIET_HOLD - Duration::from_secs(1))
            })
        {
            let next = self.plan.pop_front().expect("front plan exists");
            if let Completion::Ready(clip) = next.completion {
                actions.push(FaviconAction::Publish {
                    deck: clip.job.deck,
                    project_id: self.project_id(clip.job.deck).into(),
                    path: clip.job.output.clone(),
                });
                self.publishing = Some(clip);
            } else {
                self.allow(&mut actions);
            }
        }
        Ok(actions)
    }

    /// Cancel work, release audio and blank every configured deck before shutdown.
    pub fn stop(&mut self) -> Vec<FaviconAction> {
        let mut actions = Vec::new();
        self.blank_all(&mut actions, true);
        actions
    }

    fn dual(&self) -> bool {
        self.config
            .deck_b_project_id
            .as_ref()
            .is_some_and(|id| *id != self.config.project_id)
    }

    fn active_decks(&self) -> Vec<Deck> {
        if self.dual() {
            vec![Deck::A, Deck::B]
        } else {
            vec![Deck::A]
        }
    }

    fn project_id(&self, deck: Deck) -> &str {
        match deck {
            Deck::A => &self.config.project_id,
            Deck::B => self
                .config
                .deck_b_project_id
                .as_deref()
                .expect("deck B requires project"),
        }
    }

    fn other(&self, deck: Deck) -> Deck {
        if self.dual() && deck == Deck::A {
            Deck::B
        } else {
            Deck::A
        }
    }

    fn set_front(&mut self, front: Option<Deck>, actions: &mut Vec<FaviconAction>) {
        if self.front != front {
            self.front = front;
            actions.push(FaviconAction::SetFront(front));
            actions.push(FaviconAction::SetLive(front.is_some()));
        }
    }

    fn drop_plan(&mut self, actions: &mut Vec<FaviconAction>) {
        if !self.plan.is_empty() || self.publishing.is_some() {
            let busy = self.plan.iter().any(|p| p.running);
            self.plan.clear();
            self.publishing = None;
            actions.push(FaviconAction::CancelPending);
            if busy {
                actions.push(FaviconAction::ReplaceEncoder);
            }
            self.allow(actions);
        }
    }

    fn allow(&self, actions: &mut Vec<FaviconAction>) {
        let files = self
            .active_decks()
            .into_iter()
            .filter_map(|deck| {
                self.decks[deck.index()]
                    .clip
                    .as_ref()
                    .map(|c| &c.job.output)
            })
            .chain(self.publishing.iter().map(|c| &c.job.output))
            .chain(self.plan.iter().map(|p| &p.job.output))
            .filter_map(|path| path.file_name().map(PathBuf::from))
            .collect();
        actions.push(FaviconAction::SetAllowedFiles(files));
    }

    fn release_audio(&mut self, actions: &mut Vec<FaviconAction>) {
        if self.gate.take().is_some() {
            actions.push(FaviconAction::ReleaseAudio { at: None });
        }
    }

    fn restart(&mut self, actions: &mut Vec<FaviconAction>) {
        self.release_audio(actions);
        self.set_front(None, actions);
        self.drop_plan(actions);
        for deck in &mut self.decks {
            deck.clip = None;
            deck.published_at = None;
        }
        self.allow(actions);
    }

    fn blank_all(&mut self, actions: &mut Vec<FaviconAction>, force: bool) {
        let changed = force
            || self.gate.is_some()
            || self.front.is_some()
            || !self.plan.is_empty()
            || self.publishing.is_some()
            || self
                .active_decks()
                .iter()
                .any(|deck| !self.decks[deck.index()].blank);
        if !changed {
            return;
        }
        self.release_audio(actions);
        self.set_front(None, actions);
        self.drop_plan(actions);
        for deck in self.active_decks() {
            let state = &mut self.decks[deck.index()];
            state.clip = None;
            state.published_at = None;
            if force || !state.blank {
                state.blank = true;
                actions.push(FaviconAction::Blank {
                    deck,
                    project_id: self.project_id(deck).into(),
                    path: self.config.clips_dir.join("glowtube-blank.svg"),
                });
            }
        }
        self.allow(actions);
    }

    fn plan_next(
        &mut self,
        track: &TrackInfo,
        mut now: f64,
        wall: Instant,
        gate_allowed: bool,
        actions: &mut Vec<FaviconAction>,
    ) {
        if self.plan.len() >= 2 {
            return;
        }
        let previous = if let Some(last) = self.plan.back() {
            Some((
                last.job.start.as_secs_f64(),
                (last.job.start + last.job.duration).as_secs_f64(),
                last.job.deck,
                now.max(last.job.done_by.as_secs_f64()),
            ))
        } else if self.front_clip().is_some() {
            self.active_decks()
                .into_iter()
                .filter_map(|deck| {
                    self.decks[deck.index()]
                        .clip
                        .as_ref()
                        .filter(|clip| clip.job.track_id == track.id)
                        .map(|clip| (deck, clip))
                })
                .max_by_key(|(_, clip)| clip.job.start)
                .map(|(deck, clip)| {
                    (
                        clip.job.start.as_secs_f64(),
                        clip.end().as_secs_f64(),
                        deck,
                        now,
                    )
                })
        } else {
            None
        };
        let Some((prev_start, prev_end, deck, ready_at)) = previous else {
            if now > track.frames as f64 / f64::from(FPS) - 1.0 {
                return;
            }
            if self.config.gate && gate_allowed {
                if self.gate.is_none() {
                    self.gate_sequence += 1;
                    let gate = Gate {
                        request_id: self.gate_sequence,
                        deadline: wall + GATE_MAX,
                        position: None,
                        release_at: None,
                    };
                    actions.push(FaviconAction::HoldAudio {
                        request_id: gate.request_id,
                        deadline: gate.deadline,
                    });
                    self.gate = Some(gate);
                }
                let Some(position) = self.gate.as_ref().and_then(|gate| gate.position) else {
                    return;
                };
                now = position.as_secs_f64();
                self.queue(
                    track,
                    Deck::A,
                    now,
                    self.config.first_clip.as_secs_f64(),
                    true,
                    now,
                    actions,
                );
            } else {
                self.queue(
                    track,
                    Deck::A,
                    now + 3.0,
                    self.config.first_clip.as_secs_f64(),
                    true,
                    now,
                    actions,
                );
            }
            return;
        };
        if prev_end >= track.frames as f64 / f64::from(FPS) - 0.5 {
            return;
        }
        let handover = if self.dual() {
            3.0f64.min(0.4 * (prev_end - prev_start))
        } else {
            1.0
        };
        let publish_lead = self.config.show_latency.as_secs_f64() + 0.85;
        let start = (prev_end - handover)
            .max(ready_at + publish_lead + encode_seconds(handover + 3.0, true));
        let budget = start - publish_lead - ready_at;
        let fast = (budget - ENCODE_START) / GOOD_COST < 10.0;
        let seconds = ((budget - ENCODE_START) / if fast { FAST_COST } else { GOOD_COST })
            .min(self.config.clip_seconds.as_secs_f64() + handover)
            .max(handover + 3.0);
        self.queue(
            track,
            self.other(deck),
            start,
            seconds,
            fast,
            ready_at,
            actions,
        );
    }

    #[allow(clippy::too_many_arguments)] // Direct port of Python _queue's measured scheduling inputs.
    fn queue(
        &mut self,
        track: &TrackInfo,
        deck: Deck,
        start: f64,
        seconds: f64,
        fast: bool,
        ready_at: f64,
        actions: &mut Vec<FaviconAction>,
    ) {
        let seconds = seconds.min(track.frames as f64 / f64::from(FPS) - start);
        if seconds < 1.0 {
            return;
        }
        self.sequence += 1;
        let job = ClipJob {
            track_id: track.id.clone(),
            video: track.path.clone().expect("loaded source"),
            deck,
            start: Duration::from_secs_f64(start),
            duration: Duration::from_secs_f64(seconds),
            output: self.config.clips_dir.join(format!(
                "glowtube-clip-{}-{}-{}.svg",
                deck.name(),
                self.config.run_id,
                self.sequence
            )),
            speed: if fast { 10 } else { 8 },
            quality: if fast { 55 } else { 60 },
            done_by: Duration::from_secs_f64(ready_at + encode_seconds(seconds, fast)),
        };
        actions.push(FaviconAction::Encode(job.clone()));
        self.plan.push_back(Planned {
            job,
            completion: Completion::Waiting,
            running: false,
        });
        self.allow(actions);
    }
}

fn encode_seconds(seconds: f64, fast: bool) -> f64 {
    ENCODE_START + seconds * if fast { FAST_COST } else { GOOD_COST }
}
