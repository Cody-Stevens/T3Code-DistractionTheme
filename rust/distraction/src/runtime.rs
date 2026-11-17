//! Native CLI adapters. Tests inject dependencies into the owners instead of using these entry points.
use crate::{
    control::{ControlServer, handle_request},
    favicon::{FaviconDriver, NativeEffects},
    player::{AddMode, Player},
    write_loop::{WriteLoop, WriteTiming},
};
use dt_core::{CardTarget, Error, Result, constants::*};
use dt_platform::input::{self, InputEvent};
use dt_render::{
    card::{CardContent, CardRenderOptions},
    encode::FfmpegAvifEncoder,
};
use dt_t3::{
    T3Paths,
    theme::{ThemeDocument, ThemeWriter},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// Logs, media cache, clips and encoder scratch stay on the local system drive.
pub fn state_dir() -> Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .or_else(|| std::env::var_os("HOME"))
        .map(|root| PathBuf::from(root).join("GlowTube"))
        .ok_or(Error::InvalidInput("home directory unavailable"))
}
/// Native tools/cache configuration, resolved without creating files.
pub fn download_config() -> Result<dt_media::DownloadConfig> {
    Ok(dt_media::DownloadConfig {
        yt_dlp: "yt-dlp".into(),
        ffmpeg: "ffmpeg".into(),
        cache_dir: state_dir()?.join("cache"),
    })
}
/// Separate seekable encode scratch directory inside local state.
pub fn encoder() -> Result<FfmpegAvifEncoder> {
    Ok(FfmpegAvifEncoder {
        ffmpeg: "ffmpeg".into(),
        work_dir: state_dir()?.join("encode"),
    })
}
/// System fonts are supplied by path rather than embedded.
pub fn card_options() -> CardRenderOptions {
    CardRenderOptions {
        fonts_dir: std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/Windows"))
            .join("Fonts"),
    }
}
/// Honor Python's port override while keeping the host fixed to 127.0.0.1.
pub fn control_port() -> Result<u16> {
    let port = match std::env::var("GLOWTUBE_PORT") {
        Ok(value) => value
            .parse()
            .map_err(|_| Error::InvalidInput("invalid control port"))?,
        Err(_) => CONTROL_PORT,
    };
    if port == 0 {
        Err(Error::InvalidInput("control port must be nonzero"))
    } else {
        Ok(port)
    }
}
/// Install a process-wide Ctrl-C flag; teardown happens on the owner thread.
pub fn termination_flag() -> Result<Arc<AtomicBool>> {
    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    if let Some(flag) = FLAG.get() {
        return Ok(flag.clone());
    }
    let flag = Arc::new(AtomicBool::new(false));
    let shutdown = flag.clone();
    ctrlc::set_handler(move || shutdown.store(true, Ordering::Release))
        .map_err(|_| Error::Protocol("could not register shutdown handler"))?;
    let _ = FLAG.set(flag.clone());
    Ok(flag)
}
fn idle_document() -> Result<ThemeDocument> {
    let card = dt_render::card::render(
        &CardContent {
            title: "Nothing playing".into(),
            subtitle: "Copy a YouTube link, then press Play".into(),
            playing: false,
            upcoming: vec![],
        },
        &card_options(),
    )?;
    ThemeDocument::glowtube(dt_render::css::idle_theme(&card)?)
}
/// Explicit idle command: write an idle theme; do not select it or mutate projects.
pub fn idle() -> Result<()> {
    let paths = T3Paths::discover()?;
    ThemeWriter {
        themes_dir: paths.themes_dir,
    }
    .write(&idle_document()?)
}
struct Permissions;
impl dt_t3::setup::TokenPermissions for Permissions {
    fn restrict_to_owner(&self, path: &Path) -> Result<()> {
        dt_platform::process::restrict_to_owner(path)
    }
}
/// Explicit setup adapter: scoped token, pinned projects, complete idle theme and selection.
pub fn setup_with_paths(paths: T3Paths) -> Result<()> {
    let root = std::env::var_os("DISTRACTION_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or(state_dir()?.join("workspace"));
    let config = dt_t3::setup::SetupConfig {
        paths,
        workspace_root: root,
        idle_theme: idle_document()?,
    };
    let report = dt_t3::setup::setup_decks(&config, &Permissions)?;
    if report.idle_threads_required {
        eprintln!(
            "Both favicon decks need idle sidebar threads; the current T3 session could not create them. See dt-t3 setup guidance."
        );
    }
    Ok(())
}
fn favicon(paths: T3Paths) -> Result<FaviconDriver> {
    let api = dt_t3::api::T3Api::new(paths)?;
    let project = api
        .find_project("GlowTube", None)?
        .ok_or(Error::Protocol("GlowTube project missing; run setup"))?;
    let project_b = api
        .find_project("GlowTube B", None)?
        .map(|project| project.id);
    let clips_dir = state_dir()?.join("clips");
    let latency = std::env::var("GLOWTUBE_FAVICON_LATENCY")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .and_then(|s| Duration::try_from_secs_f64(s).ok())
        .unwrap_or(FAVICON_SHOW_LATENCY);
    let config = dt_t3::favicon::FaviconConfig {
        clips_dir: clips_dir.clone(),
        project_id: project.id.clone(),
        deck_b_project_id: project_b,
        run_id: format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ),
        show_latency: latency,
    };
    FaviconDriver::new(
        Box::new(dt_t3::favicon::FaviconScheduler::new(config)),
        Arc::new(NativeEffects {
            encoder: encoder()?,
            api,
            clips_dir,
        }),
    )
}
/// Apply verified platform events; clipboard text is scanned only on a copied-link action.
pub fn input_event(
    player: &mut Player,
    event: InputEvent,
    clipboard: impl FnOnce() -> Result<Option<String>>,
) -> Result<()> {
    use crate::control::command;
    match event {
        InputEvent::Toggle => {
            command(player, "toggle", "")?;
        }
        InputEvent::Next => {
            command(player, "next", "")?;
        }
        InputEvent::Previous => {
            command(player, "prev", "")?;
        }
        InputEvent::Card(hit) => match hit.target {
            CardTarget::Toggle => {
                command(player, "toggle", "")?;
            }
            CardTarget::Scrub => player.seek_to(f64::from(hit.fraction))?,
            CardTarget::QueueRow(row) => {
                player.play_next(usize::from(row))?;
            }
            CardTarget::LinkPlay | CardTarget::LinkQueue => {
                let text: String = clipboard()?
                    .unwrap_or_default()
                    .chars()
                    .take(4000)
                    .collect();
                let link = text
                    .split_whitespace()
                    .map(|s| {
                        s.trim_matches(|c| {
                            matches!(
                                c,
                                '<' | '>' | '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';'
                            )
                        })
                    })
                    .find(|s| {
                        reqwest::Url::parse(s).ok().is_some_and(|url| {
                            url.scheme() == "https"
                                && url.host_str().is_some_and(|host| {
                                    host == "youtube.com"
                                        || host.ends_with(".youtube.com")
                                        || host == "youtu.be"
                                })
                        })
                    });
                if let Some(link) = link {
                    player.add(
                        link.into(),
                        if hit.target == CardTarget::LinkPlay {
                            AddMode::Now
                        } else {
                            AddMode::End
                        },
                    )?;
                } else {
                    player.say("Copy a YouTube link first".into(), Duration::from_secs(4));
                }
            }
        },
    }
    player.dirty = true;
    Ok(())
}
/// Own all native resources, dispatch commands and wait on audio-grid deadlines with wakeups.
pub fn play(urls: Vec<String>) -> Result<()> {
    let Some(_instance) = dt_platform::process::single_instance("Local\\GlowTubePlayer")? else {
        return Err(Error::Protocol("player already running"));
    };
    let shutdown = termination_flag()?;
    // Fail on an occupied port before opening devices or touching T3 state.
    let (send, receive) = mpsc::channel();
    let mut server = ControlServer::start(
        std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, control_port()?),
        send,
    )?;
    let mut player = Player::new(urls)?;
    let mut writes = WriteLoop::new(player.clock(), WriteTiming::default())?;
    let mut favicon = match favicon(T3Paths::discover()?) {
        Ok(driver) => Some(driver),
        Err(_) => {
            eprintln!("favicon unavailable; theme video remains active");
            None
        }
    };
    let mut watchers = vec![];
    let (events, input_events) = mpsc::channel();
    let input_stop = Arc::new(AtomicBool::new(false));
    if let Err(error) = input::enable_dpi_awareness() {
        eprintln!("input setup failed: {error}");
    }
    let fill = std::env::var("GLOWTUBE_CLICK_FILL")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
        .unwrap_or(0.32);
    match input::watch_clicks(
        input::ClickConfig {
            minimum_fill: fill,
            bar_tolerance: 14,
        },
        input_stop.clone(),
        events.clone(),
    ) {
        Ok(watcher) => watchers.push(watcher),
        Err(_) => eprintln!("click watcher unavailable"),
    }
    match input::watch_media_keys(input_stop.clone(), events) {
        Ok(watcher) => watchers.push(watcher),
        Err(_) => eprintln!("media keys unavailable"),
    }
    let mut next_favicon = Instant::now();
    let result = (|| -> Result<()> {
        while player.running && !shutdown.load(Ordering::Acquire) {
            for request in receive.try_iter().take(64) {
                handle_request(&mut player, request);
            }
            if !player.running {
                break;
            }
            for event in input_events.try_iter().take(64) {
                if let Err(error) = input_event(&mut player, event, input::read_clipboard_text) {
                    eprintln!("input failed: {error}");
                }
            }
            player.poll()?;
            // Synchronize source revisions before the scheduler can request a fresh quiet batch.
            writes.synchronize(&player)?;
            if Instant::now() >= next_favicon {
                if let Some(favicon) = favicon.as_mut()
                    && let Err(error) = favicon.tick(&mut player, &mut writes, Instant::now())
                {
                    eprintln!("favicon tick failed: {error}");
                }
                next_favicon = Instant::now() + Duration::from_millis(100);
            }
            if let Err(error) = writes.tick(&mut player) {
                eprintln!("theme write failed: {error}");
            }
            let wait = writes
                .delay(&player)
                .min(next_favicon.saturating_duration_since(Instant::now()))
                .clamp(Duration::from_millis(1), Duration::from_millis(50));
            match receive.recv_timeout(wait) {
                Ok(request) => handle_request(&mut player, request),
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(_) => break,
            }
        }
        Ok(())
    })();
    input_stop.store(true, Ordering::Release);
    for watcher in &mut watchers {
        let _ = watcher.stop();
    }
    let server_result = server.stop();
    let player_result = player.stop();
    let favicon_result = favicon.as_mut().map(FaviconDriver::stop).unwrap_or(Ok(()));
    let write_result = writes.stop();
    // Final idle write also respects the most recent debounce/quiet deadline.
    writes.finish_idle()?;
    result
        .and(server_result)
        .and(player_result)
        .and(favicon_result)
        .and(write_result)
}
