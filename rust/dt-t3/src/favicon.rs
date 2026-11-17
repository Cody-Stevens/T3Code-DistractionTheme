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
    /// Full duration, including the overlap (up to 48 seconds).
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
    /// Encoding completed.
    Encoded(ReadyClip),
    /// Encoding failed. Python consumes the failed result when publication becomes due.
    EncodeFailed(ClipJob),
    /// Publication acknowledged. Python considers the clip live when the request is issued.
    Published(PathBuf),
    /// Publication failed. Preserve Python's optimistic live state; see the documented flaw.
    PublishFailed(PathBuf),
}

/// Player snapshot. Call about every 100 ms and deliver completions exactly once.
pub struct FaviconTick<'a> {
    /// Loaded track; Some implies PCM is ready. A missing source path disables favicon video.
    pub track: Option<&'a TrackInfo>,
    /// Audible media clock, frozen while paused and discontinuous after seeks.
    pub media_time: Duration,
    /// Injected monotonic wall time; tests advance an Instant without sleeping.
    pub wall_time: Instant,
    /// False invalidates outstanding jobs and blanks both decks.
    pub playing: bool,
    /// False for favicon off.
    pub enabled: bool,
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
    sequence: u64,
    track_id: Option<String>,
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
            sequence: 0,
            track_id: None,
            quiet_requested_at: None,
            latest_grant: None,
            last_wall: None,
        }
    }

    /// Current CSS front deck, including Python's optimistic cold-start publication state.
    pub fn front(&self) -> Option<Deck> {
        self.front
    }

    /// Clip the scheduler believes is on top; use end() and show latency to assess coverage.
    pub fn front_clip(&self) -> Option<&ReadyClip> {
        self.front
            .and_then(|deck| self.decks[deck.index()].clip.as_ref())
    }

    /// Tick the measured 6 -> ~13 -> ~33 -> 45-second ramp (plus handover overlap).
    /// Preserve Python's 600 ms late cutoff, adaptive encode budget and optimistic API state.
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
        let track = input
            .track
            .filter(|track| track.path.is_some() && input.playing && input.enabled);
        let Some(track) = track else {
            self.blank_all(&mut actions, false);
            self.track_id = None;
            return Ok(actions);
        };
        let now = input.media_time.as_secs_f64();
        if self.track_id.as_deref().is_some_and(|id| id != track.id) {
            self.blank_all(&mut actions, false);
        }
        self.track_id = Some(track.id.clone());
        if self.front_clip().is_some_and(|clip| {
            clip.job.track_id != track.id
                || now < clip.job.start.as_secs_f64() - 0.6
                || input.media_time >= clip.end()
        }) {
            self.blank_all(&mut actions, false);
        }
        if self
            .plan
            .front()
            .is_some_and(|p| p.job.track_id != track.id || now > p.job.start.as_secs_f64() + 0.6)
        {
            self.drop_plan(&mut actions);
        }
        for event in input.events {
            match event {
                FaviconEvent::Encoded(clip) => {
                    if let Some(planned) = self.plan.iter_mut().find(|p| p.job == clip.job)
                        && matches!(planned.completion, Completion::Waiting)
                    {
                        planned.completion = Completion::Ready(clip);
                    }
                }
                FaviconEvent::EncodeFailed(job) => {
                    if let Some(planned) = self.plan.iter_mut().find(|p| p.job == job)
                        && matches!(planned.completion, Completion::Waiting)
                    {
                        planned.completion = Completion::Failed;
                    }
                }
                // Python's _publish catches errors then assigns deck.clip regardless of success.
                FaviconEvent::Published(_) | FaviconEvent::PublishFailed(_) => (),
            }
        }
        for deck in self.active_decks() {
            let state = &self.decks[deck.index()];
            if Some(deck) != self.front
                && state.clip.as_ref().is_some_and(|clip| {
                    clip.job.track_id == track.id
                        && clip.job.start <= input.media_time
                        && input.media_time < clip.end()
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
        self.plan_next(track, now, &mut actions);
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
                let deck = clip.job.deck;
                actions.push(FaviconAction::Publish {
                    deck,
                    project_id: self.project_id(deck).into(),
                    path: clip.job.output.clone(),
                });
                self.decks[deck.index()] = DeckState {
                    clip: Some(clip),
                    published_at: Some(input.wall_time),
                    blank: false,
                };
                if self.front.is_none() || !self.dual() {
                    self.set_front(Some(deck), &mut actions);
                }
                actions.push(FaviconAction::Cleanup {
                    clips_dir: self.config.clips_dir.clone(),
                    keep: RETAIN_CLIPS,
                });
            }
        }
        Ok(actions)
    }

    /// Cancel all work and blank each configured deck before shutdown.
    pub fn stop(&mut self) -> Vec<FaviconAction> {
        let mut actions = Vec::new();
        self.blank_all(&mut actions, true);
        self.track_id = None;
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
        if !self.plan.is_empty() {
            self.plan.clear();
            actions.push(FaviconAction::CancelPending);
        }
    }

    fn blank_all(&mut self, actions: &mut Vec<FaviconAction>, force: bool) {
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
    }

    fn plan_next(&mut self, track: &TrackInfo, now: f64, actions: &mut Vec<FaviconAction>) {
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
            self.queue(track, Deck::A, now + 3.0, 6.0, true, now, actions);
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
            .min(45.0 + handover)
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
        });
    }
}

fn encode_seconds(seconds: f64, fast: bool) -> f64 {
    ENCODE_START + seconds * if fast { FAST_COST } else { GOOD_COST }
}
