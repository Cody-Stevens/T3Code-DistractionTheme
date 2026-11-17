# Rust rewrite plan

The Python helper in `python/` works and is the specification. The Rust version replaces it with a
single small binary that is easy to install, starts with T3 Code, and is easy for agents to change.
Read [AGENTS.md](../AGENTS.md) first: it lists the T3 Code behaviours everything depends on.

## Goals

- One binary, `distraction.exe`, with subcommands: `play [urls]`, `ctl <command> [arg]`,
  `setup`, `service` (supervisor), `idle`.
- Same behaviour and on-screen result as the Python helper, measured with the scripts in
  `python/tools/` (they work against any helper on port 47873).
- Lower CPU and memory than Python, no Python runtime. External tools allowed at runtime:
  `ffmpeg`, `ffprobe`, `yt-dlp` (and Node.js for yt-dlp). Everything else is linked in.
- Windows first (Win32 input). Keep platform code behind one crate so macOS/Linux can follow.

## Workspace

`rust/` is a Cargo workspace. Crates and owners:

| Crate | Responsibility | Python reference |
|---|---|---|
| `dt-core` | Shared types and constants: FPS, sizes, `TrackInfo`, `AudioFeatures`, `Frame` (RGB24), `Clock` trait, error type. No heavy dependencies. | constants at the top of `glowtube.py` |
| `dt-media` | yt-dlp metadata/playlist expansion and download cache; ffmpeg audio decode to PCM; per-frame audio analysis (16 log bands, bass envelope, kick, shockwave radius); video frame reader (ffmpeg pipe, seek, look-ahead buffer); audio output with a sample-accurate clock (cpal or WASAPI). | `expand`, `load`, `analyse`, `VideoReader`, `Player._callback`, `media_time` |
| `dt-render` | Everything that produces pixels or CSS: animated AVIF encoding (short theme clips with size targeting; long favicon clips wrapped in SVG, padded past 4 MiB, exact 24 fps frame durations), glow frames, edge colors, card image (2x, 16-color lossless WebP, marker colors exact), and the theme CSS generator. | `encoder.py`, `glow_frame`, `edge_colors`, `card_image.py`, `theme_css.py`, `build_dynamic` |
| `dt-t3` | T3 Code integration: runtime file and origin, bearer token file, HTTP API client (projects list/create/update), theme file writer (atomic replace), theme selection in settings.json, first-run setup (token via `t3 auth session issue`, project, theme), favicon video scheduler (clip ramp, quiet windows, cleanup). | `t3api.py`, `write_theme`, `service.setup`, `FaviconVideo` |
| `dt-platform` | Windows: DPI awareness, foreground-process check, left-click watcher with pixel strip reads (GDI), clipboard text, media-key hotkeys, process liveness, single-instance mutex, child process tree kill. | `watch_clicks`, `read_click`, `read_clipboard_text`, `watch_media_keys`, `service.pid_alive` |
| `distraction` (bin) | Orchestration: queue and lazy loader, the 250 ms write loop on the audio-clock grid, encode-ahead pool, card state, commands, loopback control server (127.0.0.1:47873, same commands and JSON as Python), service supervisor (start with T3, stop with T3, restart with back-off), CLI. | `Player`, `play`, `command`, `serve`, `service.serve` |

Crates depend downward only: `distraction` -> {`dt-media`, `dt-render`, `dt-t3`, `dt-platform`} -> `dt-core`.

## Quality bars (measure, don't assume)

- Theme writes: <= 49,500 CSS characters, every 250 ms on the audio grid; build time median under
  100 ms, worst under 200 ms (`python/tools/batch_sizes.py` shows the Python numbers).
- AVIF: Chromium (Edge headless and T3's Electron) must **animate** it (some encoders' output only
  shows the still item; see AGENTS.md). Quality at equal size must be at least Pillow's libavif
  speed 10; speed-6-class quality for encode-ahead clips. Compare with `python/tools/encoder_match.py`.
- Favicon video covers the player 100% of the time music plays, across start, seeks, pause/resume
  and track changes (`python/tools/overlay_chart.py` with `GLOWTUBE_MEASURE=1`); audio/video offset
  within 0.25 s for >= 95% of samples (`python/tools/sync_full.py`); T3's own call latency under
  0.3 s median while playing (`python/tools/t3_latency.py`). See AGENTS.md, "Favicon video".
- Click detection: 28/28 targets at 100% and 150% scale, <= 1/400 false hits on video
  (`python/tools/click_test.py`).
- Card image <= 9 KB base64; markers pixel-exact after encoding.

## Rules for every contributor

- Work only in your crate unless the integration owner agrees; change `dt-core` only through a
  small PR that the others can rebase on.
- Commit `Cargo.lock`. Prefer pure-Rust crates; if a native library is needed (libavif/libaom,
  libwebp), it must build with the MSVC toolchain and CMake already on the machine, or be vendored
  with a documented reason.
- No secrets in code, tests or logs; the T3 token is read from
  `~\.secrets\t3code-distraction-theme\t3-token` and never printed.
- Don't touch the running helper, the user's T3 settings, themes or projects while developing.
  Test against a temporary themes directory and a mock HTTP server unless a task says otherwise.
- `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test` pass before a PR.
- Don't credit an agent, a model or AI in commits, PRs, docs or code.
