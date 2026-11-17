# Distraction orchestration

This crate owns queue state, background loading, theme scheduling, control requests, favicon effects,
input dispatch and the service child. It compiles against the public crate contracts on `main`.
The other four crates are still stubs in this branch's base: native `play`, `idle`, `setup` and
`service` require their implementations before use. `ctl` works with the running Python helper.

## Timing and ownership

`Player` serializes commands and queue changes. Ordered metadata expansion runs on one worker;
a second worker loads only the current and next entries. Every queue entry has an internal identity
so duplicate URLs, insertion, reordering and a late completion cannot attach audio to the wrong entry.
PCM outside those two positions is evicted; cached paths and metadata remain. The audio callback
belongs to dt-media. Only short control/clock operations take the output handle's mutex.

`WriteLoop` predicts the displayed frame from the audible clock, smoothed build time and signed
offset. Ordinary batches contain eight frames and advance by six frames at 24 fps. Within two
frames of the next scheduled start it stays on that grid; seeks and larger drift start a fresh grid.
`until_next` measures the remaining audio time, bounded to 0..500 ms. A separate monotonic clock
enforces the 150 ms write gap and explicit holds. HTTP commands can wake the owner without bypassing
those holds. A fallback/loading/paused retry has a bounded deadline when there is no advancing grid.

Six independent ahead workers prepare five upcoming starts. Admission is bounded to twelve jobs,
duplicates are rejected, and each encode reserves 600 base64 characters. Lookup prefers the exact
start, then +/-1 frame; the batch uses the result's actual start. Source revisions reject late worker
results. The final dt-render builder rechecks the available room and owns the immediate ladder/still
fallback. Cheap mode applies beneath a live favicon and for long quiet batches.

A quiet request is pending until a successful complete 60-frame cheap write lands. An ordinary,
paused, idle, failed, duplicate or truncated-tail write cannot grant it. That successful write opens
a 2.2-second hold and grants publication 450 ms after the write. The favicon bridge checks those
bounds before enqueueing a publication, and its API worker rejects work that starts after the hold.
A failed build or write leaves the request pending. Writes resume after the hold without requiring
a command. Each write includes the selected front-deck rule. `GLOWTUBE_MEASURE` adds Python's magenta
coverage marker to dynamic writes.

Card cache keys include title, effective subtitle, playing state and four visible upcoming rows.
The transient message expires for card rendering but remains in status JSON, matching Python.
Favicon encoding and API effects have separate workers so neither queues behind theme encoding.
Shutdown joins workers/watchers, blanks favicon decks after old API effects finish, and writes idle
only after the most recent debounce/quiet deadline.

## Commands and local state

The CLI exposes `play [urls]`, `ctl [command] [arg]`, `setup`, `service` and `idle`.
The loopback server accepts GET requests on exactly `127.0.0.1`, normally port 47873; `GLOWTUBE_PORT`
overrides the port for the player, service and client. Tests always allocate ephemeral ports.
Unknown commands return 404; invalid numeric arguments return 400. The response fields are exactly
`track`, `playing`, `status`, `time`, `queue`, `index`, `volume`, `chaos`, `offset`, `quality`.
`status` does not wake rendering. URL arguments use the same form-encoded `v` parameter as Python.
The listener has bounded headers and interruptible reads/reply waits, and owns/joins its socket
thread rather than leaving detached HTTP workers running.

Windows state stays under `%LOCALAPPDATA%/GlowTube` (home-directory fallback on other platforms):

- `service.log` and append-only `player.log`;
- `cache/`, `clips/` and encoder scratch `encode/`;
- `workspace/`, the default setup project root; `DISTRACTION_WORKSPACE_ROOT` overrides it.

T3/token paths and their environment overrides remain owned by dt-t3. Explicit setup supplies the
idle document and dt-platform's owner-only permission adapter. `idle` writes a theme without
selecting it. Global media keys and verified card hits use the same commands as HTTP; clipboard
reads happen only when a copied-link button is clicked.

The service holds `Local\GlowTubeService`; the player separately holds `Local\GlowTubePlayer`.
It polls the PID in `userdata/server-runtime.json` every three seconds and requires actual process
liveness. It restarts at `min(60, 2^crashes)` seconds, resets after five calm minutes, and retries
setup/start failures after 30 seconds. Stopping requests `ctl quit` with a three-second timeout,
waits up to ten seconds for exit, then kills and reaps the owned process tree. Ctrl-C requests orderly
teardown on the owner thread.

## Validation and remaining integration

From `rust/`, set `CARGO_BUILD_JOBS=4` for every Cargo invocation. `cargo fmt --all --check`,
`cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo build --locked --workspace`
and `cargo test --locked --workspace` pass. The suite contains 27 distraction tests and six core
tests; the orchestration tests finish in about 0.1 seconds on this machine. No test needs a media
tool, device, real token, live T3 files or port 47873.

Fake clocks verify the 250 ms grid, eight-frame batches, 150 ms minimum gap, signed prediction,
.7/.3 build smoothing, 60-frame quiet batches, exact 450 ms/2.2 s deadlines, automatic resumption,
and rejected ordinary/failed/truncated acknowledgements. Synthetic media tests verify current/next
residency, stale load/encode rejection, EOF/wrap, playlist insertion/reordering and bounded worker
shutdown. Ephemeral HTTP tests verify JSON fields, query encoding, unknown routes and shutdown.
A fake runtime file and fake child verify PID checks, back-off/reset/retry and graceful-to-tree-kill
ordering without starting processes. Fake platform events and a fake favicon scheduler/API exercise
the dispatch and quiet-publication boundary.

The only shared API change is the first commit, `b3f6eb0`: `dt_render::css::DynamicInput.extra_css`.
The renderer must put these complete rules in the head before calculating video room. This budgets
the front-deck selector and measurement marker within the 49,500-character CSS limit.

| Quality bar from RUST_PLAN.md | Evidence in this branch |
| --- | --- |
| 250 ms audio grid, eight frames, >=150 ms write gap | Exact fake-clock assertions; wall time alone cannot advance the normal grid. |
| CSS <=49,500 characters | An oversized fake theme is rejected before writing; final video fitting is dt-render's responsibility. |
| Build median <100 ms, worst <200 ms | Not measured with real codecs; sibling implementations are absent. Fake-clock cost tests verify prediction/smoothing only. |
| Animated AVIF and quality versus Pillow | Deferred to dt-render and Chromium verification after integration. |
| Favicon >=96% visibility, A/V offset <=0.2 s | Quiet handshake tested; live coverage/sync measurements deferred. |
| Click targets 28/28 at both scales, <=1/400 false hits | Verified input-event dispatch only; real GDI detection belongs to dt-platform. |
| Card <=9 KB base64, pixel-exact markers | Card state/cache wiring implemented; codec/pixel measurements belong to dt-render. |

Before native/live acceptance, rebase onto the other crate implementations and resolve their API
changes. In particular, the skeleton scheduler only exposes one project in its config and no
explicit front-deck action; reconcile its final dual-deck contract with `FaviconDriver` (which can
route a/b filenames to both projects and flip 1.2 seconds after publication). Check the setup owner
creates both deck roots/projects and supplies idle threads. Confirm cancellation/timeouts in native
media/encoder adapters, then measure real build costs, AVIF animation/quality, coverage, A/V sync,
card bytes/markers and scaled click detection using `python/tools/`. The current native adapters
are wired to the contracts, but this branch alone cannot replace Python.
