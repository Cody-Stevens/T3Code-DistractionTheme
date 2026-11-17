# Rust workspace

Phase one defines the six crate boundaries in [the Rust plan](../docs/RUST_PLAN.md).
`dt-core` and the `distraction ctl` HTTP client work. Media, rendering, T3 integration, platform
operations and orchestration are documented interfaces with `todo!()` bodies. The other CLI
subcommands return `not implemented yet` before opening devices, binding ports or changing files.
Do not use this skeleton as a replacement for the Python player yet.

## Build and check

Use stable Rust with the MSVC toolchain and Visual Studio C++ build tools on Windows. The workspace
declares Rust 1.88 as its minimum; CI uses current stable. Build output stays in `rust/target/`,
which the repository ignores. Do not set `CARGO_TARGET_DIR` to another drive.

Run these commands from `rust/`:

```powershell
cargo build --locked
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked
cargo doc --locked --workspace --no-deps
```

The tests exercise validated core buffers/features and `ctl status` against an isolated ephemeral
loopback mock. They do not read a T3 token, open audio, touch user files, or connect to port 47873.
The explicit live smoke test below is separate from `cargo test`.

For a release executable, run `cargo build --locked --release -p distraction`.
It produces `target/release/distraction.exe`. Thin LTO and one codegen unit favor a compact binary
without removing unwinding from normal builds. Shared Rust/Clippy lints apply to every crate.
Unsafe code is denied by default; the platform owner may scope a reviewed override to a small
Win32 FFI module when implementing it. Do not disable the lint across the workspace.

## Run the control client

With the existing Python helper running, the read-only smoke test is:

```powershell
cargo run --locked -p distraction -- ctl status
```

This sends `GET http://127.0.0.1:47873/status?v=` with a five-second timeout and prints the helper's
JSON. `ctl` without a command also selects `status`. `GLOWTUBE_PORT` or `ctl --port <port> status`
overrides the port while keeping the host fixed to 127.0.0.1. The client disables environment
proxies and redirects and does not use a T3 bearer token. Unknown response fields are preserved.

The commands match `glowtube.command`: `status`, `toggle`, `play`, `pause`, `next`, `prev`,
`seek`, `vol`, `chaos`, `offset`, `add`, `queue`, `playnext`, `favicon`, `holdwrites`, `quit`.
An optional second argument becomes the form-encoded query parameter `v`. Negative arguments work
(for example a negative relative seek). Every command except `status` changes the running helper;
development/live verification for this phase uses only `ctl status`.

The remaining CLI surface is already reserved by clap:

```text
distraction play [URL_OR_FILE ...]
distraction setup
distraction service
distraction idle
```

These commands currently fail explicitly without invoking their implementation stubs. Calling a
`todo!()` library API directly will panic; it is an implementation boundary, not a working fallback.

## Crate map and integration contracts

Each engineer owns one crate directory. Keep imports downward: the binary depends on the four
implementation crates; each implementation crate depends only on `dt-core` among workspace crates.
No sibling implementation crate depends on another. Propose a small core PR for shared changes.

| Crate | Public entry points | Reference and owner boundary |
| --- | --- | --- |
| `dt-core` | `constants`, `Frame`, `TrackInfo`, `AudioFeatures`, `AudioFeatureFrame`, `Clock`, `Error`, `Result`, `CardTarget` | Shared values only; std with no external dependencies. |
| `dt-media` | `expand`, `load`, `decode_audio`, `analyse`, `PcmAudio`, `LoadedTrack`, `VideoSource`/`VideoReader`, `AudioOutput`/`CpalOutput` | `glowtube` acquisition, FFT, video pipe, PCM callback and audible clock. |
| `dt-render` | `light`, `edge_colors`, `glow_frame`; `encode::{AvifEncoder,FfmpegAvifEncoder,encode_to_fit,encode_video,encode_clip_svg}`; `card::{render,encode}`; `css::{static_css,build_dynamic,idle_theme,...}` | All image bytes and CSS; no T3 files/API or device ownership. |
| `dt-t3` | `T3Paths`, `read_runtime`, `api::T3Api`, `theme::{ThemeDocument,ThemeWriter,select_theme}`, `setup`, `favicon::FaviconScheduler` | Runtime discovery, scoped API, atomic files, setup, favicon scheduling. |
| `dt-platform` | `input::{enable_dpi_awareness,read_click,read_clipboard_text,watch_clicks,watch_media_keys}`, `process::{pid_alive,single_instance,spawn_hidden,kill_process_tree,restrict_to_owner}` | Win32 handles, physical pixels, hooks/clipboard and owned process cleanup. |
| `distraction` | `cli`, `control`, `player`, `write_loop`, `ahead_pool`, `supervisor` | Queue, command dispatch, adapters, worker ownership and orderly shutdown. |

Every stub's rustdoc names its Python source and states its inputs, outputs or lifecycle contract.
The generated docs (`target/doc/distraction/index.html`) include the full signatures and fields.
The APIs use these conventions:

- `Frame::new(width, height, bytes)` validates exactly width*height*3 row-major RGB24 bytes.
  Its dimensions/length remain private; a mutable pixel slice cannot invalidate its shape. Decode,
  glow and card sizes can all use the same type. Use `Arc<Frame>` when sharing immutable frames.
- `TrackInfo` holds metadata and complete 24 fps frame count. Queue/load state stays in the binary;
  `LoadedTrack` holds `Arc<PcmAudio>` and `Arc<AudioFeatures>`. Publish ready PCM only after the
  complete load succeeds. Keep only the current/next tracks' PCM resident.
- `PcmAudio.samples` contains stereo `[i16; 2]` sample frames at 48 kHz. `AudioFeatures::new`
  validates equally sized bands/bass/kick/ring arrays, finite normalized envelopes and nonnegative
  finite ring radii. `get(k)` returns a copy of one frame's values; empty analysis is valid.
- `Clock: Send + Sync` returns audible media `Duration`, adjusted for output latency. Pause freezes
  it; seeks and track changes can move it backward. Use `Instant` separately for wall-time holds.
  The callback must not allocate or run tool/HTTP work. cpal stream ownership may live on a device
  thread, with a thread-safe clock/control handle exposed through `AudioOutput`.
- `dt-render` owns base64/codec details. `BuiltTheme` returns the full stylesheet plus video room
  for ahead sizing. `DynamicState` is owned only by the write loop; ahead workers return results
  keyed by track/start rather than changing global renderer state.
- `FaviconScheduler::tick` takes a player/clock snapshot and encode/publish `FaviconEvent` results,
  then returns `FaviconAction` values. The binary adapts `ClipJob` to the render request, executes
  API publishes, handles encode/publish failures, and acknowledges quiet writes. The scheduler
  cannot encode or sleep inside a tick. A dedicated favicon worker stays separate from short clips.
- `ControlServer::start` takes a sender for `ControlRequest` messages. The player handles each
  request and sends one `ControlStatus` result back through its reply channel; the listener owns
  HTTP responses without sharing a player mutex with slow work.
- `setup::TokenPermissions` is injected by the binary through dt-platform's owner-only ACL API.
  This lets setup retain its security contract without a dt-t3 -> dt-platform dependency.
- Input events contain core `CardTarget` values, not renderer types. Marker colors also live in
  core so rendering and GDI reads agree exactly. Threads/watchers must stop, release handles and
  join on shutdown/drop; kill owned process trees rather than only their parent shells.
- Shared errors use static context/status codes and exclude HTTP bodies, bearer headers and tool
  output. `T3Api` deliberately has no Debug/Serialize implementation. Never log request objects.

## Measured budgets

[AGENTS.md](../AGENTS.md) and the Python implementation remain the behavioral specification.
These limits are part of the public API contracts, not evidence that the stubs meet them:

| Component | Contract |
| --- | --- |
| Writes | Every 250 ms on a six-frame audio grid; eight 24 fps frames, including two spares. At least 150 ms between writes around the 100 ms trailing debounce. |
| CSS | Complete standalone stylesheet each write, <=49,500 Unicode characters. New animation names; no backslashes in declaration values; quoted data URLs only. |
| Build time | Median <100 ms, worst <200 ms. Six encode workers prepare five writes/1.25 s ahead; allow +/-1 frame lookup and reserve 600 base64 characters. |
| Video/glow | Decode 640x360 RGB24; display 800x450 CSS pixels; glow 128x72 stretched 4x. Hidden theme video uses cheap encoding while favicon is live. |
| Audio | 48 kHz stereo, 16 log bands, 24 fps features; speaker clock is authoritative. Ring travels at 1440 screen px/s. |
| Card | 276 CSS pixels wide, rasterized at 2x, normally 16-color palette (32/64 fallback), lossless WebP <=9,000 base64 characters; exact marker RGB colors. |
| Favicon | 8 -> 24 -> 45 s ramp plus one-second overlap, unique run+sequence names, retain 30 clips, pad SVG to >=4,300,000 bytes. |
| Swap window | Write 2.5 s of content, hold writes 2.2 s, publish after 450 ms settle; default API/show delay 500 ms. |
| Timing | Favicon frame durations use cumulative rounding to integer milliseconds (41/42 ms), totaling exactly 1000 ms/24 frames; short Python theme clips use 42 ms/frame. |
| Acceptance | Chromium must animate AVIF; equal-size quality >=Pillow speed 10, ahead speed-6 class. Favicon visible >=96%, A/V offset <=0.2 s. Clicks 28/28 at 100/150% scale, <=1/400 false hits on video. |

Run the reference measurement tools under `python/tools/` when the implementation is ready and
live testing is authorized. Unit tests and a valid container header alone cannot prove animation,
quality, overlay visibility, sync, or click accuracy.

## Dependency and AVIF decisions

Each crate manifest records why it needs each dependency. cpal 0.16 supplies Windows WASAPI output;
rustfft supplies the 4096-sample Hann FFT. `image` has default codecs disabled, `fontdue` rasterizes
installed fonts, and pure-Rust `image-webp` supplies lossless RGB encoding. Palette reduction must
separately lock/verify the marker colors; losslessness cannot undo a quantizer changing them.
reqwest's blocking client avoids an application async executor; rustls avoids OpenSSL installation.
The Windows-only `windows` dependency exposes the Win32 feature groups used by dt-platform.

Animated AVIF needs an animation container as well as AV1 frames. `ravif`'s still-image API is not
an adequate replacement for Pillow's sequence encoder. The intended backend is external ffmpeg
with `libaom-av1` and the `avif` muxer; both are present on this Windows machine, confirmed with
`ffmpeg -h encoder=libaom-av1` and `ffmpeg -h muxer=avif` on 2026-11-16.

The inspected native alternative was `libavif-sys 0.18.0+libavif.1.4.2`, with defaults disabled and
`codec-aom` enabled, plus `libaom-sys 0.18.1+libaom.3.15.3`. Its vendored
`cmake/aom_configure.cmake` requires NASM/YASM for x86 assembly, while its Rust build script exposes
no assembly-disable switch. This machine has MSVC/CMake but no NASM, YASM or clang on PATH.
The stock optimized native path therefore needs additional tools; disabling assembly or patching
the build is a separate performance/build investigation. No native build was attempted or tool
installed. The backend interface allows replacing ffmpeg with a proven native implementation later.

The render owner must implement and measure the ffmpeg path as follows:

1. Feed equal-sized RGB24 frames without a shell, encode `libaom-av1`, disable still-picture mode
   for sequences, and use the AVIF muxer. Write to a seekable private temporary file: this muxer
   needs seeking. Reap child processes and delete only their owned scratch files.
2. Preserve requested per-frame durations with the muxer's 1000-unit movie timescale and verify
   them with ffprobe. For favicon clips, cumulative millisecond rounding must not drift.
3. Map `AvifOptions.plays=1` to ffmpeg's **single-play** loop setting (its loop=0 means infinite).
   Do not assume Pillow's loop argument has identical numeric semantics.
4. Calibrate CRF/CPU-used against the reference. The installed ffmpeg exposes CPU-used 0..8;
   Pillow quality and speed=6/8/10 are quality intents in this API, not interchangeable numbers.
5. Prove multi-frame animation in Chromium/T3 and compare at matched size with
   `python/tools/encoder_match.py`. Until then the ffmpeg backend remains a stub, not a validated
   codec replacement. Process overhead may require a native backend to meet the write budget.

Playback will require ffmpeg/ffprobe, yt-dlp and Node.js on PATH, as allowed by the plan. No external
media tool is needed to build, run core/mock tests or send `ctl status`.

## CI and security

[The Windows workflow](../.github/workflows/rust.yml) builds, checks fmt, runs Clippy with warnings
denied (including tests), and tests using the committed Cargo.lock. It declares `contents: read`,
disables checkout credential persistence and pins both actions to full commit SHAs resolved with
`gh api` (security SOP CI-1/CI-2, DEP-1). No deployment or production credentials are used.
Review dependency updates with a minimum three-day release age; Cargo has no general install-script
disable switch, so native build scripts need review. The selected build needs no secret injection.

This change defines token/path APIs without reading or creating secrets, environments or user state.
The eventual T3 token remains scoped to orchestration:read/operate in the documented owner-only
external path; setup must enforce ACLs before exposing it (SEC-3/SEC-6/SEC-8). Tests of T3 writers
and the API must use temporary directories and a mock server. No infrastructure register or other
repository is changed by this phase.
