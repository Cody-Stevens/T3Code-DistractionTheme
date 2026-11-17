# Distraction orchestration

This crate owns queue state, background loading, theme scheduling, control requests, favicon effects,
input dispatch and the service child. It compiles against the public crate contracts on `main`.
Native media/audio, rendering, Win32 input/process ownership, T3 API/setup, theme writes and the
two-deck scheduler use the implementations merged on `main` through `4c19038`. Test doubles remain
behind the injected boundaries. `play` runs the native pipeline; `ctl` speaks the same protocol as
the Python helper. No owner follow-up is required to wire the current interfaces.

## Timing and ownership

`Player` serializes commands and queue changes. Ordered metadata expansion runs on one worker;
a second worker loads only the current and next entries. Every queue entry has an internal identity
so duplicate URLs, insertion, reordering and a late completion cannot attach audio to the wrong entry.
PCM outside those two positions is evicted; cached paths and metadata remain. The audio callback
belongs to dt-media. Only short control/clock operations take the output handle's mutex.

`WriteLoop` predicts the displayed frame from the audible clock, smoothed build time and signed
offset. Ordinary batches contain eight frames and advance by six frames at 24 fps. Within two
frames of the next scheduled start it stays on that grid; seeks and larger drift start a fresh grid.
Under a live favicon, ordinary writes advance by 48 frames (two seconds), carrying 60 frames of
glow and values but just one hidden video frame. `until_next` measures the remaining audio time,
bounded to twice the active step (500 ms uncovered, four seconds covered). A separate monotonic clock
enforces the 150 ms write gap and explicit holds. HTTP commands can wake the owner without bypassing
those holds. A fallback/loading/paused retry has a bounded deadline when there is no advancing grid.

Six independent ahead workers prepare five upcoming starts. Admission is bounded to twelve jobs,
duplicates are rejected, and each encode reserves 600 base64 characters. Lookup prefers the exact
start, then +/-1 frame; the batch uses the result's actual start. Source revisions reject late worker
results. The final dt-render builder rechecks the available room and owns the immediate ladder/still
fallback. Cheap mode applies beneath a live favicon and for long quiet batches.

A quiet request tags the batch before building it. Moving audio produces up to 60 cheap frames;
covered video is a still while glow and values retain their frames. Held/paused audio produces a
still at the exact media position with no grid advancement or display prediction. Only the tagged
batch grants a window; ordinary writes never grant one. An unchanged tagged batch can grant without
rewriting, including when the decoder has no frames yet and the current theme is retained. Initial
idle fallbacks, build errors and failed writes cannot grant. A grant opens a 2.2-second hold and
permits publication after 450 ms. The real scheduler receives the landed time and adds the settle
delay itself; the bridge checks the final permission deadline. The favicon bridge checks those
bounds before enqueueing a publication, and its API worker rejects work that starts after the hold.
A failed build or write leaves the request pending. Writes resume after the hold without requiring
a command. Empty batches preserve the current theme while the decoder catches up. Each write
includes dt-render's selected front-deck opacity rule and allowed-file filter. Its measure/loading
diagnostics honor `GLOWTUBE_MEASURE` and `GLOWTUBE_SHOW_LOADING`. Before reading current or ahead
frames, `VideoSource::begin_batch(k0)` anchors dt-media's retention rule at the write's start.

Card cache keys include title, effective subtitle, playing state and four visible upcoming rows.
The transient message expires for card rendering but remains in status JSON, matching Python.
Favicon encoding and API effects have separate workers so neither queues behind theme encoding.
The real scheduler receives the player's jump counter, reported device latency, exact frozen
position after `AudioHeld`, completed effects and landed quiet-write time. Holds arm an independent
six-second audio failsafe. A successful first publication schedules release with device latency
subtracted; release never changes the user's paused state. Status queries the actual audio gate.

Theme and ahead clips use the native libavif backend. Long favicon jobs run in one warmed hidden
child of this executable, owned by a dt-platform Job Object. Small bounded JSON requests carry
local paths and timing; frames stay inside the child. `ReplaceEncoder` kills and reaps its whole
codec/ffmpeg tree before joining and warming the replacement. Generation checks reject obsolete
jobs before process creation. A start barrier makes `EncodeStarted`, scheduler cancellation and
worker replacement atomic with respect to queued work. The child never discovers T3 state,
reads a token, opens an audio device or binds a control port.
Shutdown joins workers/watchers, blanks favicon decks after old API effects finish, and writes idle
only after the most recent debounce/quiet deadline.

## Commands and local state

The CLI exposes `play [urls]`, `ctl [command] [arg]`, `setup`, `service` and `idle`.
The loopback server accepts GET requests on exactly `127.0.0.1`, normally port 47873; `GLOWTUBE_PORT`
overrides the port for the player, service and client. Tests always allocate ephemeral ports.
Unknown commands return 404; invalid numeric arguments return 400. The response fields are exactly
`track`, `playing`, `status`, `time`, `queue`, `index`, `volume`, `chaos`, `offset`, `quality`,
`held`, `front`. `front` is `"a"`, `"b"`, or null; `held` is independent of the playing flag.
`status` does not wake rendering. URL arguments use the same form-encoded `v` parameter as Python.
The listener has bounded headers and interruptible reads/reply waits, and owns/joins its socket
thread rather than leaving detached HTTP workers running.

Windows state stays under `%LOCALAPPDATA%/GlowTube` (home-directory fallback on other platforms):

- `service.log` and append-only `player.log`;
- `cache/` and `clips/`;
- `workspace/`, the default setup project root; `DISTRACTION_WORKSPACE_ROOT` overrides it.

T3/token paths and their environment overrides remain owned by dt-t3. Explicit setup supplies the
idle document and dt-platform's owner-only permission adapter. `setup_decks` also creates an idle
sidebar thread per deck when the T3 session supports the required MCP flow, without sending an
agent message. Existing generic bearer tokens can require manual thread creation; setup reports
that condition. `idle` writes a theme without
selecting it. Global media keys and verified card hits use the same commands as HTTP; clipboard
reads happen only when a copied-link button is clicked.

The service uses dt-platform's `Local\GlowTubeDistractionService` mutex; the player separately holds
`Local\GlowTubePlayer`. The Rust service mutex differs from Python's during migration.
It polls the PID in `userdata/server-runtime.json` every three seconds and requires actual process
liveness. It restarts at `min(60, 2^crashes)` seconds, resets after five calm minutes, and retries
setup/start failures after 30 seconds. Stopping requests `ctl quit` with a three-second timeout,
waits up to ten seconds for exit, then kills and reaps the owned process tree. Even graceful root
exit releases the platform Job Object and waits for descendants. Ctrl-C requests orderly
teardown on the owner thread.

## Validation

From `rust/`, set `CARGO_BUILD_JOBS=4` for every Cargo invocation. `cargo fmt --all --check`,
`cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo build --locked --workspace`
and `cargo test --locked --workspace` pass. There are 39 distraction unit tests and a default
integration test that warms, kills and restarts the real codec child. Tests open no audio device,
read no real token, modify no live T3 state and never bind port 47873.

`cargo build --locked --release -p distraction` produces `target/release/distraction.exe`.
An additional isolated test generates three seconds of ffmpeg lavfi video/audio, loads and analyses
it through dt-media, and encodes a 12-frame padded favicon through the real child and native codec.
It passed when run explicitly with ffmpeg/ffprobe on PATH:

```powershell
cargo test --locked -p distraction --test native -- --ignored --nocapture
```

The test owns and removes its temporary media and kills/reaps its processes. It is ignored by
default because external tools are not guaranteed in CI.

Fake clocks verify the 250 ms grid, eight-frame batches, 150 ms minimum gap, signed prediction,
.7/.3 build smoothing, 60-frame quiet batches, exact 450 ms/2.2 s deadlines, automatic resumption,
and rejected ordinary/failed acknowledgements. Covered and held batches, unchanged quiet grants,
empty-batch preservation, front/allowed-file rules and the seek counter have regression tests.
Synthetic media tests verify current/next
residency, stale load/encode rejection, EOF/wrap, playlist insertion/reordering and bounded worker
shutdown. Ephemeral HTTP tests verify JSON fields, query encoding, unknown routes and shutdown.
A fake runtime file and fake child verify PID checks, back-off/reset/retry and graceful-to-tree-kill
ordering without starting processes. Fake platform events and a fake favicon scheduler/API exercise
the dispatch and quiet-publication boundary. The real ThemeWriter is exercised in a temporary
directory, and NativeEffects sends real T3 API requests to an ephemeral mock with a fake token.

The only shared API change is the separate first commit: `dt_render::css::DynamicInput.extra_css`.
The renderer puts these complete rules in the head before calculating video room. This budgets
the front-deck selector and allowed-file filter within the 49,500-character CSS limit.

| Quality bar from RUST_PLAN.md | Evidence in this branch |
| --- | --- |
| 250 ms uncovered / 2 s covered audio grid, >=150 ms write gap | Exact fake-clock assertions; wall time alone cannot advance the grid. |
| CSS <=49,500 UTF-16 units | Oversized themes are rejected before writing; native renderer tests include still video, all 60 glow/value frames, and per-write clip rules. The real ThemeWriter independently validates the document. |
| Build median <100 ms, worst <200 ms | dt-render reports 91.7/113.0 ms over 20 release builds with a cached card. Full player timing under six-worker contention remains unmeasured; cold cards and difficult fallback scenes can exceed this budget. |
| Animated AVIF and quality versus Pillow | The native encoder is wired in. dt-render records Edge animation and matched-size PSNR comparisons in its README; the synthetic child test verifies native padded SVG output. |
| Favicon 100% coverage, 95% of A/V offsets within 0.25 s, T3 latency <0.3 s median | Quiet handshake and covered cadence tested; live measurements remain. |
| Click targets 28/28 at both scales, <=1/400 false hits | dt-platform's synthetic 100%/150% target and 400-click noise tests pass in the workspace suite; input dispatch is tested here. Live GDI measurements remain. |
| Card <=9 KB base64, pixel-exact markers | dt-render reports 7,764 base64 characters for its four-row Segoe sample; all marker pixels survive its round-trip tests. The native card/cache path is wired here. |

The native scheduler supplies explicit project IDs, deck/front actions, retention counts and blank
paths; the bridge no longer infers projects from filenames or implements its own deck-flip timer.
Setup uses `setup_decks` and reports when idle sidebar threads could not be provisioned.

## Differences from Python

- Setup roots its projects at `%LOCALAPPDATA%/GlowTube/workspace` by default rather than the
  Python source directory. Set `DISTRACTION_WORKSPACE_ROOT` to that directory to reuse the same
  project roots. Setup additionally attempts idle-thread creation through dt-t3.
- The service mutex is `Local\GlowTubeDistractionService`, as defined by dt-platform, rather
  than Python's `Local\GlowTubeService`. The player has its own mutex and refuses an occupied
  control port. Switch services explicitly during migration.
- The measured cadence is fixed at 48 covered frames and a 2.2-second quiet hold; Python's
  `GLOWTUBE_COVERED_STEP` and `GLOWTUBE_QUIET_HOLD` tuning overrides are not implemented.
  Port, volume, gate, first/max clip lengths, show latency and measure/loading switches are wired.
- Service logs contain event text without Python's timestamp/PID/exit-code detail.
  `GLOWTUBE_DEBUG`'s detailed timing and ahead-counter logs are not ported.
- HTTP input and errors are bounded/redacted. Invalid numeric arguments return 400 rather than
  dropping the Python request handler. Successful routes and the twelve status fields match.
- Native font rasterization/palette reduction can differ from Pillow. dt-render retains a fitted
  fallback level rather than trying one level better synchronously on every write; ahead workers
  explore higher quality. Its generic libaom ahead encodes and cold card generation can cost more
  than Python's optimized build; see the renderer's measurements.

No production test doubles or orchestration stubs remain. Live build costs, favicon coverage,
A/V sync, T3 call latency and scaled clicks still need measurement using `python/tools/` after
the player swap. No playback, setup, idle or service command was run against the live T3 setup.
