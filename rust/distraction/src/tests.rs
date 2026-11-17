//! Synthetic integration tests. No test binds 47873 or reads/writes user T3 state.
use crate::{
    ahead_pool::{AheadEncoder, AheadPool, ClipKey},
    control::{self, ControlClient, ControlServer},
    player::{AddMode, LoadState, MediaLoader, Player},
    supervisor::{Supervisor, SupervisorHost},
    write_loop::{RenderBatch, Renderer, ThemeSink, WallClock, WriteLoop, WriteTiming},
};
use dt_core::{
    AudioFeatureFrame, AudioFeatures, Clock, Error, Frame, Result, TrackInfo, constants::*,
};
use dt_media::{AudioOutput, ExpandedTrack, LoadedTrack, PcmAudio, VideoSource};
use dt_render::{card::CardContent, css::BuiltTheme, encode::EncodedVideo};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct AudioState {
    millis: AtomicU64,
    ended: AtomicBool,
    playing: AtomicBool,
    stopped: AtomicBool,
    source: Mutex<Option<Arc<PcmAudio>>>,
}
struct FakeAudio(Arc<AudioState>);
impl Clock for AudioState {
    fn media_time(&self) -> Duration {
        Duration::from_millis(self.millis.load(Ordering::SeqCst))
    }
}
impl Clock for FakeAudio {
    fn media_time(&self) -> Duration {
        self.0.media_time()
    }
}
impl AudioOutput for FakeAudio {
    fn set_source(&mut self, source: Option<Arc<PcmAudio>>) -> Result<()> {
        *self.0.source.lock().unwrap() = source;
        self.0.millis.store(0, Ordering::SeqCst);
        self.0.ended.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn set_playing(&mut self, playing: bool) -> Result<()> {
        self.0.playing.store(playing, Ordering::SeqCst);
        Ok(())
    }
    fn seek(&mut self, position: Duration) -> Result<()> {
        self.0
            .millis
            .store(position.as_millis() as u64, Ordering::SeqCst);
        Ok(())
    }
    fn set_volume(&mut self, _: f32) -> Result<()> {
        Ok(())
    }
    fn ended(&self) -> bool {
        self.0.ended.load(Ordering::SeqCst)
    }
    fn stop(&mut self) -> Result<()> {
        self.0.stopped.store(true, Ordering::SeqCst);
        self.0.source.lock().unwrap().take();
        Ok(())
    }
}
#[derive(Default)]
struct Loader {
    loads: Mutex<Vec<String>>,
    weak: Mutex<Vec<std::sync::Weak<PcmAudio>>>,
}
fn loaded(info: &TrackInfo) -> LoadedTrack {
    let mut info = info.clone();
    info.id = info.url.clone();
    info.title = info.url.clone();
    info.frames = 240;
    info.path = Some(PathBuf::from(&info.url));
    LoadedTrack {
        info,
        pcm: Arc::new(PcmAudio {
            samples: vec![[0, 0]; 480_000],
        }),
        features: Arc::new(
            AudioFeatures::new(
                vec![[0.0; BANDS]; 240],
                vec![0.1; 240],
                vec![0.0; 240],
                vec![0.0; 240],
            )
            .unwrap(),
        ),
    }
}
impl MediaLoader for Loader {
    fn expand(&self, url: &str) -> Result<Vec<ExpandedTrack>> {
        Ok(url
            .split(',')
            .map(|url| ExpandedTrack {
                url: url.into(),
                title: url.into(),
            })
            .collect())
    }
    fn load(&self, info: &TrackInfo) -> Result<LoadedTrack> {
        self.loads.lock().unwrap().push(info.url.clone());
        if info.url == "bad" {
            return Err(Error::Protocol("synthetic failure"));
        }
        let track = loaded(info);
        self.weak.lock().unwrap().push(Arc::downgrade(&track.pcm));
        Ok(track)
    }
}
fn eventually(mut test: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !test() {
        assert!(Instant::now() < deadline, "worker did not complete");
        thread::sleep(Duration::from_millis(1));
    }
}
fn player(urls: &[&str]) -> (Player, Arc<AudioState>, Arc<Loader>) {
    let audio = Arc::new(AudioState::default());
    let loader = Arc::new(Loader::default());
    let mut player = Player::with_dependencies(
        urls.iter().map(|s| s.to_string()).collect(),
        loader.clone(),
        Box::new(FakeAudio(audio.clone())),
    )
    .unwrap();
    if !urls.is_empty() {
        eventually(|| {
            player.poll().unwrap();
            player.ready().is_some()
                && player.queue.get(player.index + 1).is_none_or(|entry| {
                    matches!(entry.state, LoadState::Ready(_) | LoadState::Failed)
                })
        });
    }
    (player, audio, loader)
}
#[derive(Clone)]
struct FakeWall(Arc<Mutex<Instant>>);
impl FakeWall {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
    fn advance(&self, millis: u64) {
        *self.0.lock().unwrap() += Duration::from_millis(millis);
    }
}
impl WallClock for FakeWall {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}
#[derive(Clone, Debug)]
struct Build {
    count: usize,
    glows: usize,
    first: u8,
    cheap: bool,
    extra: String,
    prepared: bool,
    values: Vec<dt_render::css::CssValues>,
}
#[derive(Default)]
struct RenderState {
    builds: Mutex<Vec<Build>>,
    missing: AtomicBool,
    fail: AtomicBool,
    oversized: AtomicBool,
    unchanged: AtomicBool,
    stops: AtomicUsize,
    cost_ms: AtomicU64,
    wall: Mutex<Option<FakeWall>>,
}
struct FakeVideo(Arc<RenderState>);
impl VideoSource for FakeVideo {
    fn get(&mut self, k: u64) -> Result<Option<Arc<Frame>>> {
        if self.0.missing.load(Ordering::SeqCst) {
            return Ok(None);
        }
        Ok(Some(Arc::new(
            Frame::new(1, 1, vec![k as u8, 0, 0]).unwrap(),
        )))
    }
    fn start_at(&mut self, _: u64) -> Result<()> {
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        self.0.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct FakeRenderer(Arc<RenderState>);
impl Renderer for FakeRenderer {
    fn video(&mut self, _: &TrackInfo) -> Result<Box<dyn VideoSource>> {
        Ok(Box::new(FakeVideo(self.0.clone())))
    }
    fn glow(&mut self, frame: &Frame, _: AudioFeatureFrame) -> Result<Frame> {
        Ok(frame.clone())
    }
    fn colors(&mut self, _: &Frame) -> Result<[String; 5]> {
        Ok(std::array::from_fn(|_| "#010203".into()))
    }
    fn build(&mut self, _: CardContent, input: RenderBatch<'_>) -> Result<BuiltTheme> {
        if let Some(wall) = self.0.wall.lock().unwrap().as_ref() {
            wall.advance(self.0.cost_ms.load(Ordering::SeqCst));
        }
        if self.0.fail.load(Ordering::SeqCst) {
            return Err(Error::Protocol("synthetic build failure"));
        }
        let mut builds = self.0.builds.lock().unwrap();
        builds.push(Build {
            count: input.frames.len(),
            glows: input.glows.len(),
            first: input.frames[0].pixels()[0],
            cheap: input.cheap,
            extra: input.extra_css.into(),
            prepared: input.prepared.is_some(),
            values: input.values.to_vec(),
        });
        Ok(BuiltTheme {
            css: if self.0.oversized.load(Ordering::SeqCst) {
                "x".repeat(CSS_LIMIT + 1)
            } else {
                format!(
                    ":root{{--seq:{}}}{}",
                    if self.0.unchanged.load(Ordering::SeqCst) {
                        1
                    } else {
                        builds.len()
                    },
                    input.extra_css
                )
            },
            video_room: 30000,
        })
    }
    fn idle(&mut self, _: CardContent, extra: &str) -> Result<BuiltTheme> {
        Ok(BuiltTheme {
            css: format!(":root{{--idle:1}}{extra}"),
            video_room: 30000,
        })
    }
    fn quality(&self) -> String {
        "synthetic".into()
    }
}
#[derive(Default)]
struct SinkState {
    writes: Mutex<Vec<(Instant, String)>>,
    fail: AtomicBool,
}
struct FakeSink(Arc<SinkState>, FakeWall);
impl ThemeSink for FakeSink {
    fn write(&mut self, theme: &BuiltTheme) -> Result<()> {
        if self.0.fail.load(Ordering::SeqCst) {
            return Err(Error::Protocol("synthetic write failure"));
        }
        self.0
            .writes
            .lock()
            .unwrap()
            .push((self.1.now(), theme.css.clone()));
        Ok(())
    }
}
struct FakeEncoder;
impl AheadEncoder for FakeEncoder {
    fn encode(&self, _: &[Arc<Frame>], room: usize) -> Result<Option<EncodedVideo>> {
        assert!(room <= 29400);
        Ok(Some(EncodedVideo {
            base64: "AAAA".into(),
            width: 640,
            quality: 60,
        }))
    }
}
type LoopFixture = (WriteLoop, FakeWall, Arc<RenderState>, Arc<SinkState>);
fn write_loop(audio: Arc<AudioState>) -> LoopFixture {
    let wall = FakeWall::new();
    let renderer = Arc::new(RenderState::default());
    *renderer.wall.lock().unwrap() = Some(wall.clone());
    let sink = Arc::new(SinkState::default());
    let writes = WriteLoop::with_dependencies(
        audio,
        Arc::new(wall.clone()),
        WriteTiming {
            build_estimate: Duration::ZERO,
            display_offset: 0.0,
            ..WriteTiming::default()
        },
        Box::new(FakeRenderer(renderer.clone())),
        Box::new(FakeSink(sink.clone(), wall.clone())),
        AheadPool::with_encoder(6, Arc::new(FakeEncoder)).unwrap(),
    )
    .unwrap();
    (writes, wall, renderer, sink)
}
#[test]
fn lazy_loader_only_current_and_next_and_releases_old_pcm() {
    let (mut p, _, loader) = player(&["a,b,c,d"]);
    assert_eq!(&*loader.loads.lock().unwrap(), &["a", "b"]);
    assert!(matches!(p.queue[2].state, LoadState::Unloaded));
    p.jump(2).unwrap();
    eventually(|| {
        p.poll().unwrap();
        p.queue
            .get(3)
            .is_some_and(|t| matches!(t.state, LoadState::Ready(_)))
    });
    assert_eq!(&*loader.loads.lock().unwrap(), &["a", "b", "c", "d"]);
    assert!(
        loader.weak.lock().unwrap()[..2]
            .iter()
            .all(|weak| weak.upgrade().is_none())
    );
    p.stop().unwrap();
    assert!(
        loader
            .weak
            .lock()
            .unwrap()
            .iter()
            .all(|weak| weak.upgrade().is_none())
    );
}
#[test]
fn queue_modes_reordering_wrap_seek_eof_and_failed_track() {
    let (mut p, audio, _) = player(&["a,b,c,d"]);
    p.play_next(2).unwrap();
    assert_eq!(p.status().queue, ["a", "d", "b", "c"]);
    p.play_next(0).unwrap();
    p.play_next(usize::MAX).unwrap();
    p.jump(-1).unwrap();
    assert_eq!(p.index, 3);
    p.jump(1).unwrap();
    p.poll().unwrap();
    assert_eq!(p.index, 0);
    eventually(|| {
        p.poll().unwrap();
        p.ready().is_some()
    });
    p.seek(100.0).unwrap();
    assert!((p.status().time - 10.0).abs() < 0.01);
    p.seek(-100.0).unwrap();
    assert_eq!(p.status().time, 0.0);
    p.seek_to(0.5).unwrap();
    assert!((p.status().time - 5.0).abs() < 0.01);
    audio.ended.store(true, Ordering::SeqCst);
    p.poll().unwrap();
    assert_eq!(p.index, 1);
    p.add("e,f".into(), AddMode::Next).unwrap();
    eventually(|| {
        p.poll().unwrap();
        p.queue.len() == 6
    });
    assert_eq!(p.status().queue, ["a", "d", "e", "f", "b", "c"]);
    p.add("x,y".into(), AddMode::Now).unwrap();
    eventually(|| {
        p.poll().unwrap();
        p.queue.len() == 8
    });
    assert_eq!(p.track().unwrap().info.url, "x");
    assert!(p.playing);
    let (mut p, _, _) = player(&["bad,good"]);
    eventually(|| {
        p.poll().unwrap();
        p.track().unwrap().info.url == "good" && p.ready().is_some()
    });
}
#[test]
fn card_deadline_and_status_read_are_independent() {
    let (mut p, audio, _) = player(&["a,b,c,d,e,f"]);
    let now = Instant::now();
    p.say_at("hello".into(), Duration::from_secs(4), now);
    assert_eq!(p.card_content("clock".into(), now).subtitle, "hello");
    assert_eq!(
        p.card_content("clock".into(), now + Duration::from_secs(4))
            .subtitle,
        "clock"
    );
    assert_eq!(p.card_content("clock".into(), now).upcoming.len(), 4);
    audio.millis.store(1234, Ordering::SeqCst);
    p.dirty = false;
    assert!(control::command(&mut p, "status", "").unwrap());
    assert!(!p.dirty);
    assert_eq!(p.status().time, 1.2);
    assert_eq!(p.status().status, "hello");
}
#[test]
fn commands_defaults_clamps_and_bad_values() {
    let (mut p, _, _) = player(&[]);
    for (cmd, arg) in [
        ("vol", "2"),
        ("chaos", "99"),
        ("offset", "-0.3"),
        ("pause", ""),
        ("favicon", "off"),
    ] {
        assert!(control::command(&mut p, cmd, arg).unwrap());
    }
    assert_eq!(p.volume, 1.5);
    assert_eq!(p.chaos, 11.0);
    assert_eq!(p.status().offset, -0.14);
    assert!(!p.playing);
    assert!(!p.favicon_enabled);
    for cmd in [
        "play",
        "toggle",
        "next",
        "prev",
        "seek",
        "playnext",
        "holdwrites",
    ] {
        assert!(control::command(&mut p, cmd, "").unwrap());
    }
    assert!(p.hold_until.is_some());
    for arg in ["NaN", "inf", "secret-invalid-value"] {
        for cmd in ["vol", "chaos", "offset", "seek", "holdwrites"] {
            let error = control::command(&mut p, cmd, arg).unwrap_err();
            assert!(!error.to_string().contains(arg));
        }
    }
    assert!(control::command(&mut p, "holdwrites", "1e300").is_err());
    assert!(!control::command(&mut p, "unknown", "").unwrap());
    control::command(&mut p, "quit", "").unwrap();
    assert!(!p.running);
}
#[test]
fn grid_alignment_is_audio_driven_and_min_gap_survives_command_wakes() {
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    let (mut writes, wall, renderer, sink) = write_loop(audio.clone());
    assert!(writes.tick(&mut p).unwrap());
    assert_eq!(writes.timing.next_frame, Some(6));
    assert_eq!(writes.until_next(), Duration::from_millis(250));
    wall.advance(1000);
    assert!(!writes.tick(&mut p).unwrap()); // wall time alone cannot advance the grid
    audio.millis.store(250, Ordering::SeqCst);
    assert!(writes.tick(&mut p).unwrap());
    assert_eq!(writes.timing.next_frame, Some(12));
    p.dirty = true;
    audio.millis.store(480, Ordering::SeqCst);
    wall.advance(149);
    assert!(!writes.tick(&mut p).unwrap());
    wall.advance(1);
    assert!(writes.tick(&mut p).unwrap());
    let builds = renderer.builds.lock().unwrap();
    assert!(!builds[0].prepared);
    assert_eq!(
        builds.iter().map(|b| b.first).collect::<Vec<_>>(),
        [0, 6, 12]
    );
    assert!(builds.iter().all(|b| b.count == 8));
    assert!(builds[0].values[0].contains_key("c0"));
    assert!(!builds[0].values[1].contains_key("b0"));
    assert!(builds[0].values[2].contains_key("b0"));
    let times = sink.writes.lock().unwrap();
    assert!(
        times
            .windows(2)
            .all(|w| w[1].0.duration_since(w[0].0) >= MIN_WRITE_GAP)
    );
}
#[test]
fn quiet_window_only_after_long_success_and_exact_settle_and_hold() {
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    let (mut writes, wall, renderer, sink) = write_loop(audio);
    p.favicon_front = Some('b');
    assert!(writes.tick(&mut p).unwrap());
    assert!(writes.quiet_granted_at().is_none());
    writes.request_quiet();
    assert!(!writes.tick(&mut p).unwrap());
    wall.advance(150);
    assert!(writes.tick(&mut p).unwrap());
    let written = wall.now();
    assert_eq!(writes.quiet_granted_at(), Some(written + QUIET_SETTLE));
    assert_eq!(writes.timing.hold_until, Some(written + QUIET_HOLD));
    assert_eq!(writes.timing.next_frame, None);
    assert_eq!(renderer.builds.lock().unwrap()[1].count, 60);
    assert!(renderer.builds.lock().unwrap()[1].cheap);
    assert!(
        renderer
            .builds
            .lock()
            .unwrap()
            .iter()
            .all(|b| b.extra.contains("glowtube-clip-b-"))
    );
    wall.advance(2199);
    assert!(!writes.tick(&mut p).unwrap());
    wall.advance(1);
    assert!(writes.tick(&mut p).unwrap());
    assert_eq!(sink.writes.lock().unwrap().len(), 3);
}
#[test]
fn failed_long_build_idle_fallback_and_failed_write_never_grant() {
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    let (mut writes, wall, renderer, sink) = write_loop(audio);
    writes.request_quiet();
    renderer.fail.store(true, Ordering::SeqCst);
    assert!(writes.tick(&mut p).unwrap());
    assert!(writes.quiet_granted_at().is_none());
    renderer.fail.store(false, Ordering::SeqCst);
    wall.advance(250);
    sink.fail.store(true, Ordering::SeqCst);
    assert!(writes.tick(&mut p).is_err());
    assert!(writes.quiet_granted_at().is_none());
    assert!(writes.timing.hold_until.is_none());
    sink.fail.store(false, Ordering::SeqCst);
    assert!(writes.tick(&mut p).unwrap());
    assert!(writes.quiet_granted_at().is_some());
}
#[test]
fn ordinary_paused_and_initial_empty_batches_do_not_open_a_window() {
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    p.set_playing(false).unwrap();
    let (mut writes, wall, renderer, _) = write_loop(audio.clone());
    assert!(writes.tick(&mut p).unwrap());
    assert_eq!(renderer.builds.lock().unwrap()[0].count, 1);
    assert!(writes.quiet_granted_at().is_none());
    p.set_playing(true).unwrap();
    wall.advance(250);
    writes.synchronize(&p).unwrap();
    // A fresh loop has no existing CSS to keep when the decoder is catching up.
    let (mut writes, _, renderer, _) = write_loop(audio);
    writes.request_quiet();
    renderer.missing.store(true, Ordering::SeqCst);
    assert!(writes.tick(&mut p).unwrap());
    assert!(writes.quiet_granted_at().is_none());
    assert!(writes.timing.hold_until.is_none());
}
#[test]
fn seek_resets_grid_and_cheap_mode_and_css_cap_are_enforced() {
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    let (mut writes, wall, renderer, sink) = write_loop(audio.clone());
    p.favicon_live = true;
    assert!(writes.tick(&mut p).unwrap());
    assert!(renderer.builds.lock().unwrap()[0].cheap);
    p.seek(4.0).unwrap();
    wall.advance(150);
    assert!(writes.tick(&mut p).unwrap());
    assert_eq!(renderer.builds.lock().unwrap()[1].first, 96);
    assert!(renderer.stops.load(Ordering::SeqCst) > 0);
    renderer.oversized.store(true, Ordering::SeqCst);
    p.dirty = true;
    wall.advance(150);
    assert!(writes.tick(&mut p).is_err());
    assert_eq!(sink.writes.lock().unwrap().len(), 2);
    audio.millis.store(0, Ordering::SeqCst);
    assert!(writes.until_next() <= Duration::from_secs(4));
}
#[test]
fn tagged_quiet_batch_can_grant_at_the_track_tail() {
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    audio.millis.store(9000, Ordering::SeqCst);
    let (mut writes, _, renderer, _) = write_loop(audio);
    writes.request_quiet();
    assert!(writes.tick(&mut p).unwrap());
    assert_eq!(renderer.builds.lock().unwrap()[0].count, 24);
    assert!(writes.quiet_granted_at().is_some());
}
#[test]
fn ahead_nonblocking_matching_slack_invalidation_and_join() {
    let mut pool = AheadPool::with_encoder(1, Arc::new(FakeEncoder)).unwrap();
    let frame = Arc::new(Frame::new(1, 1, vec![0; 3]).unwrap());
    let key = ClipKey {
        track_id: "a".into(),
        start_frame: 12,
    };
    assert!(
        pool.submit(key.clone(), vec![frame.clone(); 8], 30000)
            .unwrap()
    );
    assert!(!pool.submit(key, vec![frame.clone()], 30000).unwrap());
    let mut clip = None;
    eventually(|| {
        clip = pool.take("a", 13);
        clip.is_some()
    });
    assert_eq!(clip.unwrap().key.start_frame, 12);
    pool.submit(
        ClipKey {
            track_id: "a".into(),
            start_frame: 18,
        },
        vec![frame.clone()],
        30000,
    )
    .unwrap();
    pool.clear();
    assert!(pool.take("a", 18).is_none());
    pool.submit(
        ClipKey {
            track_id: "b".into(),
            start_frame: 24,
        },
        vec![frame],
        30000,
    )
    .unwrap();
    assert!(pool.take("a", 24).is_none());
    pool.stop().unwrap();
    assert!(AheadPool::with_encoder(0, Arc::new(FakeEncoder)).is_err());
}
#[test]
fn loopback_server_routes_exact_json_and_unknown_and_query_encoding() {
    let (mut p, _, _) = player(&[]);
    p.dirty = false;
    let (send, receive) = mpsc::channel();
    assert!(
        ControlServer::start(
            std::net::SocketAddrV4::new(std::net::Ipv4Addr::UNSPECIFIED, 0),
            send.clone()
        )
        .is_err()
    );
    let mut server = ControlServer::start(
        std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0),
        send,
    )
    .unwrap();
    let port = server.address().port();
    assert_ne!(port, CONTROL_PORT);
    let client = thread::spawn(move || {
        ControlClient::new(port)
            .unwrap()
            .send(crate::cli::CtlCommand::Status, "x&y=+é")
            .unwrap()
    });
    let request = receive.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(request.command, "status");
    assert_eq!(request.argument, "x&y=+é");
    control::handle_request(&mut p, request);
    let value = client.join().unwrap();
    let mut keys = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "chaos", "front", "held", "index", "offset", "playing", "quality", "queue", "status",
            "time", "track", "volume"
        ]
    );
    assert!(!p.dirty);
    let client = thread::spawn(move || {
        reqwest::blocking::get(format!("http://127.0.0.1:{port}/unknown"))
            .unwrap()
            .status()
            .as_u16()
    });
    control::handle_request(
        &mut p,
        receive.recv_timeout(Duration::from_secs(2)).unwrap(),
    );
    assert_eq!(client.join().unwrap(), 404);
    server.stop().unwrap();
    assert!(std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok());
}
#[test]
fn server_stop_interrupts_unanswered_request() {
    let (send, receive) = mpsc::channel();
    let mut server = ControlServer::start(
        std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0),
        send,
    )
    .unwrap();
    let port = server.address().port();
    let client = thread::spawn(move || {
        ControlClient::new(port)
            .unwrap()
            .send(crate::cli::CtlCommand::Status, "")
    });
    let _request = receive.recv_timeout(Duration::from_secs(2)).unwrap();
    let start = Instant::now();
    server.stop().unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(client.join().unwrap().is_err());
}
#[derive(Default)]
struct FakeHost {
    running: bool,
    exited: bool,
    fail_setup: bool,
    starts: usize,
    stops: usize,
    setups: usize,
}
impl SupervisorHost for FakeHost {
    fn running(&mut self) -> bool {
        self.running
    }
    fn setup(&mut self) -> Result<()> {
        self.setups += 1;
        if self.fail_setup {
            Err(Error::Protocol("fake setup"))
        } else {
            Ok(())
        }
    }
    fn start(&mut self) -> Result<()> {
        self.starts += 1;
        Ok(())
    }
    fn exited(&mut self) -> Result<bool> {
        Ok(std::mem::take(&mut self.exited))
    }
    fn stop(&mut self) -> Result<()> {
        self.stops += 1;
        Ok(())
    }
    fn log(&mut self, _: &str) {}
}
#[test]
fn supervisor_start_stop_backoff_calm_reset_and_setup_retry() {
    let mut s = Supervisor::new(FakeHost::default());
    let sec = Duration::from_secs;
    s.tick(sec(0)).unwrap();
    assert_eq!(s.host.starts, 0);
    s.host.running = true;
    s.tick(sec(3)).unwrap();
    assert_eq!(s.host.starts, 1);
    let mut now = 6;
    for delay in [2, 4, 8, 16, 32, 60, 60] {
        s.host.exited = true;
        s.tick(sec(now)).unwrap();
        let starts = s.host.starts;
        s.tick(sec(now + delay - 1)).unwrap();
        assert_eq!(s.host.starts, starts);
        now += delay;
        s.tick(sec(now)).unwrap();
        assert_eq!(s.host.starts, starts + 1);
        now += 3;
    }
    s.tick(sec(now + 300)).unwrap();
    now += 303;
    s.host.exited = true;
    s.tick(sec(now)).unwrap();
    let starts = s.host.starts;
    s.tick(sec(now + 2)).unwrap();
    assert_eq!(s.host.starts, starts + 1);
    s.host.running = false;
    s.tick(sec(now + 5)).unwrap();
    assert_eq!(s.host.stops, 1);
    s.host.running = true;
    s.host.fail_setup = true;
    s.tick(sec(now + 8)).unwrap();
    let setups = s.host.setups;
    s.tick(sec(now + 37)).unwrap();
    assert_eq!(s.host.setups, setups);
    s.host.fail_setup = false;
    s.tick(sec(now + 38)).unwrap();
    assert_eq!(s.host.setups, setups + 1);
    s.shutdown().unwrap();
    assert_eq!(s.host.stops, 2);
}
#[test]
fn supervisor_reads_fake_runtime_and_requires_live_pid() {
    let path = std::env::temp_dir().join(format!(
        "distraction-runtime-{}-{}.json",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    for (body, expected) in [
        ("{}", false),
        ("bad", false),
        (r#"{"pid":0}"#, false),
        (r#"{"pid":123}"#, true),
        (r#"{"pid":4294967296}"#, false),
    ] {
        std::fs::write(&path, body).unwrap();
        assert_eq!(
            crate::supervisor::runtime_running(&path, |pid| pid == 123),
            expected
        );
    }
    std::fs::write(&path, r#"{"pid":123}"#).unwrap();
    assert!(!crate::supervisor::runtime_running(&path, |_| false));
    std::fs::remove_file(&path).unwrap();
    assert!(!crate::supervisor::runtime_running(&path, |_| true));
}

#[test]
fn stale_load_completion_does_not_restore_evicted_audio() {
    struct SlowLoader {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        weak: Arc<Mutex<Option<std::sync::Weak<PcmAudio>>>>,
    }
    impl MediaLoader for SlowLoader {
        fn expand(&self, _: &str) -> Result<Vec<ExpandedTrack>> {
            Ok(["a", "b", "c", "d"]
                .into_iter()
                .map(|s| ExpandedTrack {
                    url: s.into(),
                    title: s.into(),
                })
                .collect())
        }
        fn load(&self, info: &TrackInfo) -> Result<LoadedTrack> {
            let track = loaded(info);
            if info.url == "a" {
                *self.weak.lock().unwrap() = Some(Arc::downgrade(&track.pcm));
                self.entered.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
            Ok(track)
        }
    }
    let (entered, waiting) = mpsc::channel();
    let (release, receiver) = mpsc::channel();
    let weak = Arc::new(Mutex::new(None));
    let audio = Arc::new(AudioState::default());
    let mut p = Player::with_dependencies(
        vec!["playlist".into()],
        Arc::new(SlowLoader {
            entered,
            release: Mutex::new(receiver),
            weak: weak.clone(),
        }),
        Box::new(FakeAudio(audio)),
    )
    .unwrap();
    eventually(|| {
        p.poll().unwrap();
        p.queue.len() == 4
    });
    waiting.recv_timeout(Duration::from_secs(2)).unwrap();
    p.jump(2).unwrap();
    release.send(()).unwrap();
    eventually(|| {
        p.poll().unwrap();
        p.ready().is_some()
    });
    assert!(matches!(p.queue[0].state, LoadState::Unloaded));
    assert!(weak.lock().unwrap().as_ref().unwrap().upgrade().is_none());
}

#[test]
fn ahead_discards_in_flight_result_after_seek_and_stop_joins() {
    struct BlockingEncoder {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        stopped: Arc<AtomicBool>,
    }
    impl AheadEncoder for BlockingEncoder {
        fn encode(&self, _: &[Arc<Frame>], _: usize) -> Result<Option<EncodedVideo>> {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            self.stopped.store(true, Ordering::SeqCst);
            Ok(Some(EncodedVideo {
                base64: "old".into(),
                width: 640,
                quality: 60,
            }))
        }
    }
    let (entered, waiting) = mpsc::channel();
    let (release, receive) = mpsc::channel();
    let finished = Arc::new(AtomicBool::new(false));
    let mut pool = AheadPool::with_encoder(
        1,
        Arc::new(BlockingEncoder {
            entered,
            release: Mutex::new(receive),
            stopped: finished.clone(),
        }),
    )
    .unwrap();
    pool.submit(
        ClipKey {
            track_id: "a".into(),
            start_frame: 6,
        },
        vec![Arc::new(Frame::new(1, 1, vec![0; 3]).unwrap())],
        30000,
    )
    .unwrap();
    waiting.recv_timeout(Duration::from_secs(2)).unwrap();
    pool.clear();
    release.send(()).unwrap();
    pool.stop().unwrap();
    assert!(finished.load(Ordering::SeqCst));
    assert!(pool.take("a", 6).is_none());
}

#[test]
fn favicon_bridge_never_publishes_after_ordinary_write_or_before_settle() {
    use crate::favicon::{FaviconDriver, FaviconEffects, Scheduler};
    use dt_t3::favicon::*;
    use std::collections::VecDeque;
    struct FakeScheduler(Arc<Mutex<VecDeque<Vec<FaviconAction>>>>);
    impl Scheduler for FakeScheduler {
        fn tick(&mut self, _: FaviconTick<'_>, _: u64) -> Result<Vec<FaviconAction>> {
            Ok(self.0.lock().unwrap().pop_front().unwrap_or_default())
        }
        fn stop(&mut self) -> Vec<FaviconAction> {
            vec![FaviconAction::Blank {
                deck: Deck::B,
                project_id: "project-b".into(),
                path: "glowtube-blank.svg".into(),
            }]
        }
    }
    #[derive(Default)]
    struct Effects {
        publishes: AtomicUsize,
        blanks: AtomicUsize,
    }
    impl FaviconEffects for Effects {
        fn encode(&self, job: ClipJob) -> Result<ReadyClip> {
            Ok(ReadyClip { job, frames: 144 })
        }
        fn publish(&self, project: &str, _: &std::path::Path) -> Result<()> {
            assert_eq!(project, "project-b");
            self.publishes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn blank(&self, project: &str, _: &std::path::Path) -> Result<()> {
            assert_eq!(project, "project-b");
            self.blanks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn cleanup(&self, _: &std::path::Path, _: usize) -> Result<()> {
            Ok(())
        }
    }
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    let (mut writes, wall, _, _) = write_loop(audio);
    let actions = Arc::new(Mutex::new(VecDeque::new()));
    let effects = Arc::new(Effects::default());
    let mut favicon =
        FaviconDriver::new(Box::new(FakeScheduler(actions.clone())), effects.clone()).unwrap();
    let publish = || {
        vec![FaviconAction::Publish {
            deck: Deck::B,
            project_id: "project-b".into(),
            path: "glowtube-clip-b-test.svg".into(),
        }]
    };
    assert!(writes.tick(&mut p).unwrap());
    actions.lock().unwrap().push_back(publish());
    favicon.tick(&mut p, &mut writes, wall.now()).unwrap();
    assert_eq!(effects.publishes.load(Ordering::SeqCst), 0);
    actions
        .lock()
        .unwrap()
        .push_back(vec![FaviconAction::RequestQuiet]);
    favicon.tick(&mut p, &mut writes, wall.now()).unwrap();
    wall.advance(150);
    assert!(writes.tick(&mut p).unwrap());
    actions.lock().unwrap().push_back(publish());
    favicon.tick(&mut p, &mut writes, wall.now()).unwrap();
    assert_eq!(effects.publishes.load(Ordering::SeqCst), 0);
    wall.advance(449);
    actions.lock().unwrap().push_back(publish());
    favicon.tick(&mut p, &mut writes, wall.now()).unwrap();
    assert_eq!(effects.publishes.load(Ordering::SeqCst), 0);
    wall.advance(1);
    actions.lock().unwrap().push_back(publish());
    favicon.tick(&mut p, &mut writes, wall.now()).unwrap();
    eventually(|| effects.publishes.load(Ordering::SeqCst) == 1);
    favicon.stop().unwrap();
    assert_eq!(effects.blanks.load(Ordering::SeqCst), 1);
}

#[test]
fn platform_events_and_clipboard_link_validation_use_the_same_commands() {
    use dt_core::CardTarget;
    use dt_platform::input::{CardHit, InputEvent};
    let (mut p, _, _) = player(&[]);
    crate::runtime::input_event(&mut p, InputEvent::Toggle, || {
        panic!("no clipboard read for a media key")
    })
    .unwrap();
    assert!(!p.playing);
    crate::runtime::input_event(
        &mut p,
        InputEvent::Card(CardHit {
            target: CardTarget::LinkPlay,
            fraction: 0.0,
        }),
        || Ok(Some("https://youtube.com.evil.invalid/watch?v=x".into())),
    )
    .unwrap();
    assert!(p.queue.is_empty());
    assert_eq!(p.status().status, "Copy a YouTube link first");
    crate::runtime::input_event(
        &mut p,
        InputEvent::Card(CardHit {
            target: CardTarget::LinkQueue,
            fraction: 0.0,
        }),
        || Ok(Some("watch <https://youtu.be/example>".into())),
    )
    .unwrap();
    eventually(|| {
        p.poll().unwrap();
        !p.queue.is_empty()
    });
    assert_eq!(p.queue[0].info.url, "https://youtu.be/example");
}

#[test]
fn child_shutdown_requests_quit_then_waits_then_kills_whole_tree() {
    use crate::supervisor::{ChildShutdown, stop_owned_child};
    struct Child {
        now: Duration,
        exit_after: Option<Duration>,
        quit_ok: bool,
        events: Vec<&'static str>,
    }
    impl ChildShutdown for Child {
        fn exited(&mut self) -> Result<bool> {
            Ok(self.exit_after.is_some_and(|after| self.now >= after))
        }
        fn quit(&mut self, timeout: Duration) -> bool {
            assert_eq!(timeout, Duration::from_secs(3));
            self.events.push("quit");
            self.quit_ok
        }
        fn now(&self) -> Duration {
            self.now
        }
        fn wait(&mut self, duration: Duration) {
            self.now += duration;
        }
        fn kill_tree(&mut self) -> Result<()> {
            self.events.push("kill tree and reap");
            Ok(())
        }
    }
    let mut child = Child {
        now: Duration::ZERO,
        exit_after: None,
        quit_ok: true,
        events: vec![],
    };
    stop_owned_child(&mut child).unwrap();
    assert_eq!(child.events, ["quit", "kill tree and reap"]);
    assert_eq!(child.now, Duration::from_secs(10));
    let mut child = Child {
        now: Duration::ZERO,
        exit_after: Some(Duration::from_millis(150)),
        quit_ok: true,
        events: vec![],
    };
    stop_owned_child(&mut child).unwrap();
    assert_eq!(child.events, ["quit", "kill tree and reap"]);
    assert_eq!(child.now, Duration::from_millis(150));
    let mut child = Child {
        now: Duration::ZERO,
        exit_after: None,
        quit_ok: false,
        events: vec![],
    };
    stop_owned_child(&mut child).unwrap();
    assert_eq!(child.events, ["quit", "kill tree and reap"]);
    assert_eq!(child.now, Duration::ZERO);
    let mut child = Child {
        now: Duration::ZERO,
        exit_after: Some(Duration::ZERO),
        quit_ok: true,
        events: vec![],
    };
    stop_owned_child(&mut child).unwrap();
    assert_eq!(child.events, ["kill tree and reap"]);
}

#[test]
fn covered_batches_use_one_video_frame_and_sixty_glow_and_value_frames() {
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    p.favicon_live = true;
    let (mut writes, wall, renderer, _) = write_loop(audio.clone());
    assert!(writes.tick(&mut p).unwrap());
    {
        let builds = renderer.builds.lock().unwrap();
        let b = &builds[0];
        assert_eq!((b.count, b.glows, b.values.len()), (1, 60, 60));
        assert!(b.cheap);
        assert!(!b.prepared);
        assert!(b.values[59].contains_key("pr"));
    }
    assert_eq!(writes.timing.next_frame, Some(48));
    assert_eq!(writes.until_next(), Duration::from_secs(2));
    wall.advance(2000);
    assert!(!writes.tick(&mut p).unwrap());
    audio.millis.store(2000, Ordering::SeqCst);
    assert!(writes.tick(&mut p).unwrap());
    assert_eq!(renderer.builds.lock().unwrap()[1].first, 48);
    writes.timing.next_frame = Some(1000);
    assert_eq!(writes.until_next(), Duration::from_secs(4));
    // An uncovered grid keeps the old half-second cap.
    p.favicon_live = false;
    writes.batch(&mut p).unwrap();
    assert_eq!(writes.timing.step_frames, 6);
    writes.timing.next_frame = Some(1000);
    assert_eq!(writes.until_next(), Duration::from_millis(500));
}

#[test]
fn covered_quiet_batch_has_still_video_and_animated_glow() {
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    p.favicon_live = true;
    let (mut writes, wall, renderer, _) = write_loop(audio);
    writes.request_quiet();
    assert!(writes.tick(&mut p).unwrap());
    let builds = renderer.builds.lock().unwrap();
    assert_eq!(
        (builds[0].count, builds[0].glows, builds[0].values.len()),
        (1, 60, 60)
    );
    assert!(builds[0].cheap);
    assert_eq!(writes.quiet_written_at(), Some(wall.now()));
    assert_eq!(writes.quiet_granted_at(), Some(wall.now() + QUIET_SETTLE));
}

#[test]
fn held_audio_builds_at_the_frozen_position_without_advancing_the_grid() {
    let (mut p, audio, _) = player(&["a"]);
    p.held = true;
    p.offset = 0.8;
    audio.millis.store(1500, Ordering::SeqCst);
    let (mut writes, wall, renderer, _) = write_loop(audio);
    writes.timing.build_estimate = Duration::from_millis(900);
    assert!(writes.tick(&mut p).unwrap());
    assert_eq!(writes.timing.next_frame, None);
    assert_eq!(writes.until_next(), WRITE_INTERVAL);
    wall.advance(250);
    assert!(writes.tick(&mut p).unwrap());
    let builds = renderer.builds.lock().unwrap();
    assert!(
        builds
            .iter()
            .all(|b| b.count == 1 && b.glows == 1 && b.first == 36)
    );
    assert!(p.status().held);
    assert!(p.status().playing);
}

#[test]
fn unchanged_held_batch_grants_only_when_tagged_for_a_quiet_window() {
    let (mut p, audio, _) = player(&["a"]);
    p.held = true;
    let (mut writes, wall, renderer, sink) = write_loop(audio);
    renderer.unchanged.store(true, Ordering::SeqCst);
    assert!(writes.tick(&mut p).unwrap());
    wall.advance(250);
    assert!(!writes.tick(&mut p).unwrap());
    assert!(writes.quiet_granted_at().is_none());
    writes.request_quiet();
    wall.advance(250);
    assert!(!writes.tick(&mut p).unwrap());
    assert_eq!(sink.writes.lock().unwrap().len(), 1);
    assert_eq!(writes.quiet_granted_at(), Some(wall.now() + QUIET_SETTLE));
    assert_eq!(writes.timing.hold_until, Some(wall.now() + QUIET_HOLD));
}

#[test]
fn decoder_catchup_keeps_the_theme_and_both_favicon_rules() {
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    p.favicon_front = Some('a');
    p.favicon_allow = Some(vec![
        "glowtube-clip-a-test-1.svg".into(),
        "glowtube-clip-b-test-2.svg".into(),
    ]);
    let (mut writes, wall, renderer, sink) = write_loop(audio.clone());
    assert!(writes.tick(&mut p).unwrap());
    let css = sink.writes.lock().unwrap()[0].1.clone();
    assert!(css.contains("opacity:.01!important"));
    assert!(css.contains("glowtube-clip-a-\"]{opacity:1!important"));
    assert!(css.contains(":not([src*=\"glowtube-clip-b-test-2.svg\"])"));
    renderer.missing.store(true, Ordering::SeqCst);
    audio.millis.store(250, Ordering::SeqCst);
    wall.advance(250);
    assert!(!writes.tick(&mut p).unwrap());
    assert_eq!(sink.writes.lock().unwrap().len(), 1);
    assert_eq!(sink.writes.lock().unwrap()[0].1, css);
    assert!(writes.quiet_granted_at().is_none());
    // Python can grant a tagged retry of an already quiet, unchanged theme.
    writes.request_quiet();
    wall.advance(250);
    assert!(!writes.tick(&mut p).unwrap());
    assert!(writes.quiet_granted_at().is_some());
    assert_eq!(sink.writes.lock().unwrap().len(), 1);
}

#[test]
fn jump_counter_tracks_seeks_and_track_changes_but_not_loads_or_pause() {
    let (mut p, audio, _) = player(&["a,b,c"]);
    let initial = p.jumps;
    assert_eq!(initial, 1);
    p.poll().unwrap();
    p.set_playing(false).unwrap();
    p.set_playing(true).unwrap();
    assert_eq!(p.jumps, initial);
    p.seek(1.0).unwrap();
    p.seek_to(0.5).unwrap();
    assert_eq!(p.jumps, initial + 2);
    p.jump(1).unwrap();
    eventually(|| {
        p.poll().unwrap();
        p.ready().is_some()
    });
    assert_eq!(p.jumps, initial + 3);
    p.play_next(1).unwrap();
    assert_eq!(p.jumps, initial + 3);
    audio.ended.store(true, Ordering::SeqCst);
    p.poll().unwrap();
    assert_eq!(p.jumps, initial + 4);
    p.favicon_front = Some('b');
    assert_eq!(p.status().front.as_deref(), Some("b"));
}

#[test]
fn scheduler_bridge_receives_jump_counter_and_landed_quiet_time_and_sets_front() {
    use crate::favicon::{FaviconDriver, FaviconEffects, Scheduler};
    use dt_t3::favicon::*;
    type Snapshots = Arc<Mutex<Vec<(u64, Option<Instant>)>>>;
    struct Capture {
        snapshots: Snapshots,
    }
    impl Scheduler for Capture {
        fn tick(&mut self, input: FaviconTick<'_>, jumps: u64) -> Result<Vec<FaviconAction>> {
            self.snapshots
                .lock()
                .unwrap()
                .push((jumps, input.quiet_granted_at));
            Ok(vec![
                FaviconAction::SetFront(Some(Deck::B)),
                FaviconAction::SetLive(true),
            ])
        }
        fn stop(&mut self) -> Vec<FaviconAction> {
            vec![]
        }
    }
    struct NoEffects;
    impl FaviconEffects for NoEffects {
        fn encode(&self, _: ClipJob) -> Result<ReadyClip> {
            panic!("unexpected encode")
        }
        fn publish(&self, _: &str, _: &std::path::Path) -> Result<()> {
            panic!("unexpected publish")
        }
        fn blank(&self, _: &str, _: &std::path::Path) -> Result<()> {
            panic!("unexpected blank")
        }
        fn cleanup(&self, _: &std::path::Path, _: usize) -> Result<()> {
            panic!("unexpected cleanup")
        }
    }
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    let (mut writes, wall, _, _) = write_loop(audio);
    writes.request_quiet();
    writes.tick(&mut p).unwrap();
    let landed = wall.now();
    let snapshots = Arc::new(Mutex::new(vec![]));
    let mut driver = FaviconDriver::new(
        Box::new(Capture {
            snapshots: snapshots.clone(),
        }),
        Arc::new(NoEffects),
    )
    .unwrap();
    driver.tick(&mut p, &mut writes, wall.now()).unwrap();
    assert_eq!(snapshots.lock().unwrap()[0], (p.jumps, Some(landed)));
    assert_eq!(p.favicon_front, Some('b'));
    assert!(p.favicon_live);
    p.seek(0.0).unwrap();
    writes.synchronize(&p).unwrap();
    driver.tick(&mut p, &mut writes, wall.now()).unwrap();
    assert_eq!(snapshots.lock().unwrap()[1], (p.jumps, None));
    driver.stop().unwrap();
}

#[test]
fn real_theme_writer_persists_the_complete_orchestrated_theme_in_a_temp_directory() {
    let temp = tempfile::tempdir().unwrap();
    let themes = temp.path().join("themes");
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    p.favicon_front = Some('b');
    p.favicon_allow = Some(vec!["glowtube-clip-b-test-1.svg".into()]);
    let wall = FakeWall::new();
    let mut writes = WriteLoop::with_dependencies(
        audio,
        Arc::new(wall),
        WriteTiming::default(),
        Box::new(FakeRenderer(Arc::new(RenderState::default()))),
        Box::new(dt_t3::theme::ThemeWriter {
            themes_dir: themes.clone(),
        }),
        AheadPool::with_encoder(1, Arc::new(FakeEncoder)).unwrap(),
    )
    .unwrap();
    assert!(writes.tick(&mut p).unwrap());
    let files: Vec<_> = std::fs::read_dir(&themes).unwrap().collect();
    assert_eq!(files.len(), 1);
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(files[0].as_ref().unwrap().path()).unwrap()).unwrap();
    let css = value["css"].as_str().unwrap();
    assert!(css.contains("opacity:.01!important"));
    assert!(css.contains(":not([src*=\"glowtube-clip-b-test-1.svg\"])"));
    assert!(!css.contains("--idle"));
    writes.stop().unwrap();
}

#[test]
fn native_favicon_effects_use_explicit_projects_and_owned_paths_against_a_mock_api() {
    use crate::favicon::{FaviconEffects, NativeEffects};
    use std::io::{BufRead, Read, Write};
    let temp = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let mock = thread::spawn(move || {
        let mut requests = vec![];
        for _ in 0..2 {
            let mut connection = None;
            eventually(|| match listener.accept() {
                Ok((stream, _)) => {
                    connection = Some(stream);
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
                Err(error) => panic!("mock accept failed: {error}"),
            });
            let mut stream = connection.unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            assert_eq!(first.trim(), "POST /api/projects/mutate HTTP/1.1");
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            assert!(length > 0 && length < 10000);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            requests.push(serde_json::from_slice::<serde_json::Value>(&body).unwrap());
            drop(reader);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .unwrap();
        }
        requests
    });
    let paths = dt_t3::T3Paths {
        home: temp.path().into(),
        runtime_file: temp.path().join("runtime.json"),
        token_file: temp.path().join("fake-token"),
        themes_dir: temp.path().join("themes"),
        settings_file: temp.path().join("settings.json"),
        cli: temp.path().join("unused-t3"),
    };
    std::fs::write(
        &paths.runtime_file,
        serde_json::to_vec(&serde_json::json!({"origin": origin, "pid": 123})).unwrap(),
    )
    .unwrap();
    std::fs::write(&paths.token_file, "fake-test-token").unwrap();
    let effects = NativeEffects {
        encoder: dt_render::encode::FfmpegAvifEncoder {
            ffmpeg: "unused-ffmpeg".into(),
            work_dir: temp.path().join("encode"),
        },
        api: dt_t3::api::T3Api::new(paths).unwrap(),
        clips_dir: temp.path().join("clips"),
    };
    let clip = effects.clips_dir.join("glowtube-clip-b-test.svg");
    effects.publish("project-b", &clip).unwrap();
    let blank = effects.clips_dir.join("glowtube-blank.svg");
    effects.blank("project-a", &blank).unwrap();
    let requests = mock.join().unwrap();
    assert_eq!(requests[0]["projectId"], "project-b");
    assert_eq!(requests[0]["faviconPath"], clip.to_string_lossy().as_ref());
    assert_eq!(requests[1]["projectId"], "project-a");
    assert_eq!(requests[1]["faviconPath"], blank.to_string_lossy().as_ref());
    assert_eq!(
        std::fs::read_to_string(&blank).unwrap(),
        dt_t3::favicon::BLANK_SVG
    );
    assert!(
        effects
            .blank("project-a", &temp.path().join("foreign.svg"))
            .is_err()
    );
    assert!(effects.cleanup(temp.path(), 0).is_err());
}

#[test]
fn quit_response_is_delivered_before_server_shutdown() {
    let (mut p, _, _) = player(&[]);
    let (send, receive) = mpsc::channel();
    let mut server = ControlServer::start(
        std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0),
        send,
    )
    .unwrap();
    let port = server.address().port();
    let client = thread::spawn(move || {
        ControlClient::new(port)
            .unwrap()
            .send(crate::cli::CtlCommand::Quit, "")
    });
    control::handle_request(
        &mut p,
        receive.recv_timeout(Duration::from_secs(2)).unwrap(),
    );
    server.stop().unwrap();
    assert!(!p.running);
    assert!(client.join().unwrap().is_ok());
}

#[test]
fn more_than_one_worker_queue_of_initial_urls_preserves_order() {
    let urls: Vec<_> = (0..40).map(|i| format!("track-{i}")).collect();
    let audio = Arc::new(AudioState::default());
    let mut p = Player::with_dependencies(
        urls.clone(),
        Arc::new(Loader::default()),
        Box::new(FakeAudio(audio)),
    )
    .unwrap();
    eventually(|| {
        p.poll().unwrap();
        p.queue.len() == 40
    });
    assert_eq!(p.status().queue, urls);
    assert!(
        p.queue
            .iter()
            .skip(2)
            .all(|entry| matches!(entry.state, LoadState::Unloaded))
    );
}

#[test]
fn prediction_uses_build_estimate_and_offset_and_smooths_measured_cost() {
    let (mut p, audio, _) = player(&["a"]);
    audio.millis.store(1000, Ordering::SeqCst);
    let (mut writes, wall, renderer, _) = write_loop(audio.clone());
    writes.timing.build_estimate = Duration::from_millis(150);
    renderer.cost_ms.store(60, Ordering::SeqCst);
    assert!(writes.tick(&mut p).unwrap());
    assert_eq!(renderer.builds.lock().unwrap()[0].first, 31);
    assert_eq!(writes.timing.next_frame, Some(37));
    assert!((writes.timing.build_estimate.as_secs_f64() - 0.123).abs() < 1e-9);
    let expected = 37.0 / 24.0 - 1.0 - 0.123 - 0.16;
    assert!((writes.until_next().as_secs_f64() - expected).abs() < 1e-9);
    audio.millis.store(2000, Ordering::SeqCst);
    assert_eq!(writes.until_next(), Duration::ZERO);
    p.set_playing(false).unwrap();
    wall.advance(150);
    writes.synchronize(&p).unwrap();
    assert_eq!(writes.until_next(), WRITE_INTERVAL);
}

#[test]
fn matching_ahead_result_is_passed_to_renderer_for_its_actual_grid_frame() {
    struct SignalEncoder(mpsc::Sender<()>);
    impl AheadEncoder for SignalEncoder {
        fn encode(&self, _: &[Arc<Frame>], _: usize) -> Result<Option<EncodedVideo>> {
            self.0.send(()).unwrap();
            Ok(Some(EncodedVideo {
                base64: "AAAA".into(),
                width: 640,
                quality: 60,
            }))
        }
    }
    let (mut p, audio, _) = player(&["a"]);
    p.offset = 0.0;
    let wall = FakeWall::new();
    let renderer = Arc::new(RenderState::default());
    let sink = Arc::new(SinkState::default());
    let (send, receive) = mpsc::channel();
    let ahead = AheadPool::with_encoder(6, Arc::new(SignalEncoder(send))).unwrap();
    let mut writes = WriteLoop::with_dependencies(
        audio.clone(),
        Arc::new(wall.clone()),
        WriteTiming {
            build_estimate: Duration::ZERO,
            display_offset: 0.0,
            ..WriteTiming::default()
        },
        Box::new(FakeRenderer(renderer.clone())),
        Box::new(FakeSink(sink, wall.clone())),
        ahead,
    )
    .unwrap();
    writes.tick(&mut p).unwrap();
    for _ in 0..5 {
        receive.recv_timeout(Duration::from_secs(2)).unwrap();
    }
    // Completion notification precedes delivery by a few instructions; all workers are joined by Drop.
    eventually(|| {
        audio.millis.store(250, Ordering::SeqCst);
        let batch = writes.batch(&mut p).unwrap();
        batch.is_some() && renderer.builds.lock().unwrap().last().unwrap().prepared
    });
    assert_eq!(renderer.builds.lock().unwrap().last().unwrap().first, 6);
}
