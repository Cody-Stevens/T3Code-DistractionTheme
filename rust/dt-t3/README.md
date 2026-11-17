# dt-t3

T3 Code integration: runtime discovery, scoped bearer HTTP, atomic theme/settings files,
first-run setup and a pure two-deck favicon scheduler. Constructing paths or a scheduler does
no I/O. `T3Api::new` reads only the supplied token file; each request rereads the runtime file.
Tests use temporary directories, fake tokens and ephemeral loopback servers.

## Binary integration

`setup_decks` returns the pinned GlowTube and GlowTube B projects and their idle thread IDs.
The latter project is rooted at `workspace_root/deck-b`. The binary supplies the idle theme and implements `TokenPermissions`
through dt-platform's owner-only ACL function. `ensure_token_with` also accepts a `TokenIssuer`
for isolated tests. The default issuer runs the installed CLI with exactly these arguments:

```text
auth session issue --scope orchestration:read --scope orchestration:operate --ttl 90d --label "GlowTube helper" --subject mcp-client --json
```

Issuance has a 60-second deadline, captures bounded stdout privately and discards stderr. Setup
secures the token directory before issuance and the temporary token file before writing the
secret, then replaces the token atomically. Existing nonempty tokens are reused and restricted
again. `--subject mcp-client` enables the source-verified idle-thread API without adding any
scopes. No secret-bearing type implements Debug or Serialize; errors omit tool output, HTTP
bodies and headers. Authenticated HTTP disables environment proxies and redirects, validates an
HTTP(S) origin without credentials/path/query/fragment, and accepts only relative `/api/` paths
for ordinary JSON calls. Idle-thread setup uses a private initialized `/mcp` session at that
same validated origin, with JSON or SSE responses and transport teardown on success/error.

`ThemeWriter` writes a complete `glowtube.json` through a unique same-directory temporary file,
flushes it and replaces the destination atomically. It retries Windows sharing failures 20 times
at 5 ms intervals and reports exhaustion. Settings replacement preserves unknown JSON fields;
invalid settings are left intact. A missing settings file is initialized. Both the constructor
and writer enforce the 49,500 CSS budget and 48-character name limit, counted as UTF-16 units to
match JavaScript. The binary still owns the 250 ms audio-grid cadence and quiet holds.

Feed `FaviconScheduler::tick` loaded track metadata, audible media time, the player's explicit
`jumps` counter, monotonic wall time, output latency, worker/API completions and the latest quiet
write grant. Increment `jumps` on every seek and track change, including looping the same track.
Do not infer jumps from clip ranges. Execute actions in order:

- `HoldAudio` silences the player and freezes its clock. Return `AudioHeld` with the request ID
  and the exact frozen position; only then does the scheduler queue the first clip. Enforce the
  supplied six-second deadline in the audio owner even if no more ticks arrive. `ReleaseAudio`
  replaces that deadline with a successful publication's acknowledgment time plus
  `max(0, show_latency - output_latency)`; `None` releases immediately. A new hold replaces any
  earlier release timer. The scheduler also releases an unacknowledged hold at the deadline.
- Queue `ClipJob` on one dedicated serial encoder. Report `EncodeStarted` when work starts,
  then `Encoded` with actual frame count or `EncodeFailed`. Synchronize start notifications
  with tick/cancellation so a running job cannot appear queued when it is dropped.
- `CancelPending` cancels queued work. `ReplaceEncoder` follows when dropped work is already
  running: terminate the old worker and its decoder, create and warm a replacement, then queue
  subsequent `Encode` actions. Exact job matching rejects obsolete results.
- For `RequestQuiet`, build 2.5 seconds of cheap theme content, land the write and hold further
  writes for 2.2 seconds. Pass the **time the write landed** in `quiet_granted_at`; this crate
  adds the 450 ms settle. Python stores the already-settled time instead. A request may precede
  encode completion, but publication always requires a completed encode.
- Publish the supplied path to the supplied project and always return `Published` or
  `PublishFailed`. While awaiting acknowledgment, the scheduler retains the planned filename
  and issues no further publication. Only success replaces the deck's clip/file or sets it live.
  A failure retains the old deck and continues planning.
- Apply `SetFront` through `favicon_front_css(deck.name())`, and `SetAllowedFiles` through
  `favicon_allow_css` in every complete theme write. The latter contains filenames, not absolute
  paths: each deck's current clip plus planned/publishing clips. An empty list hides stale images.
  `SetLive` controls cheap hidden video. Seeks, track changes and expired clips restart through
  these theme rules, without publishing blanks.
- Write `BLANK_SVG` to `clips_dir/glowtube-blank.svg` before executing `Blank`. Blanks occur only
  when the video stops (pause, disabled/unavailable playback, quit). Retain the newest 30 owned
  `glowtube-clip-*.svg` files on `Cleanup`. Call `stop` and execute its effects before shutdown.

Pass distinct project IDs, an absolute clip directory and a unique run ID (a UUID is suitable).
Each filename includes deck, run and sequence. Set `first_clip` to 10 seconds, `clip_seconds` to
22 seconds and `gate` to true for Python defaults. The binary can override these from
`GLOWTUBE_FIRST_CLIP`, `GLOWTUBE_CLIP_SECONDS` and `GLOWTUBE_GATE`. The state machine reads no
environment variables. Deck B remains optional for Python's single-deck fallback; coverage
acceptance requires two decks and the gate enabled. No shared or sibling crate changed.

## Idle sidebar threads

Read-only investigation on 2026-11-16 used `t3 --help`, `t3 thread --help` and
`t3 thread start --help`. The thread commands are list/start/send/show. `start` requires both
a project and an agent message, with no create-only or no-turn flag. A one-level listing of
`C:\Users\samwi\Documents\GitHub` found this theme repository but no T3 Code source checkout.
The installed CLI launcher did identify a bundled `resources/server.asar`. Its compiled source
was inspected read-only, in memory. `AcpMcpStdioBridge-KpBqZZax.mjs` implements the HTTP `/mcp`
handshake (protocol 2025-06-18); `server-syNn-CpO.mjs` exposes `t3_thread_launch` with optional
message/attachments, and `ManagedEndpointRuntime--5XAAKR8.mjs` makes `initialMessage` optional
in `OrchestrationV2ThreadLaunchInput`. Launch without that field creates the thread without
dispatching a turn. `relayTracing-CeJ93I6Y.mjs`'s `authenticateMcpClient` additionally requires
the bearer subject `mcp-client`, which `t3 auth session issue --help` supports through `--subject`.
The ordinary HTTP orchestration snapshot routes are read-only; no thread-create REST route
was found. No mutating request was sent to the running server.

`T3Api::ensure_idle_thread` opens that HTTP MCP session, lists unsettled/unsnoozed idle threads
in the project and reuses one, or calls `t3_thread_launch` with root workspace and **no message
or attachments**. It verifies the result has `runId: null`. First-run setup does this for both
decks. Mock tests exercise JSON/SSE, session/protocol headers, reuse, absence of message dispatch
and transport teardown, including error paths. Live acceptance remains for integration.

Existing Python tokens are reused and are not silently rotated. Those tokens have the default
subject, so `/mcp` rejects them even though the projects API works. Setup then reports
`idle_threads_required: true` and no thread IDs. The same fallback covers servers without this
MCP endpoint. Each deck still needs an idle sidebar thread in that case; the binary should
surface this result. Other failures are returned as errors rather than hidden.

## Reference parity

Current Python and Rust plan full durations of 10.0, 10.839161, 25.0 and 25.0 seconds with the
default gate, 500 ms show latency and measured serial encode costs. The last two are 22 seconds
plus three seconds of overlap. Starts are 0.0, 7.0, 14.839161 and 36.839161 seconds. At most two
clips remain planned or publishing; each follows the newest planned/published clip. When the
encoder is behind, the next start moves later rather than queuing an impossible deadline.

The former optimistic publish state and undetected in-range seek behavior are removed, matching
Python #9. The first clip begins at the held media position, and music resumes when that image
shows. Quiet windows are requested about 1.75 seconds before a clip starts, once its encode is
complete or due within 0.5 seconds. A late encode drops the plan and replaces a running worker.

A six-second timeout intentionally lets music resume even if no picture arrived. Failures that
outlast that limit, the disabled gate, and single-deck mode can expose theme video. Simulated
100% favicon coverage applies to successful playback, including start, seeks, resume and track
changes; it does not claim coverage during encoder/API outages. In particular, after a failed
handover, Python waits until a poll
observes the front clip has expired before restarting and holding audio. At the 100 ms polling
cadence this leaves up to one tick uncovered (60.839161 ms in both replays). Rust preserves that
transition; the failed-publish test measures the gap and then verifies held recovery.

## Validation

Run from `rust/`, with `CARGO_BUILD_JOBS=4` and the default worktree `target`:

```text
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked -p dt-t3 -- --nocapture
python dt-t3/tests/reference_replay.py
python dt-t3/tests/reference_replay.py --failed-handover
```

The optional Python replay extracts only constants and scheduler definitions from the reference
AST. It replaces clocks, encoders, API calls and cleanup; it never imports or starts the helper.
Rust tests have no Python dependency. Its first starts, durations, publications and flips are
golden assertions in the Rust simulation.

Measured on this Windows worktree with debug tests:

| Applicable quality bar | Result and method |
| --- | --- |
| CSS <=49,500, name <=48 | Boundary tests accept the limit, reject one unit over it, and check astral Unicode as two UTF-16 units. |
| Atomic theme writes | Concurrent reader sees complete JSON through 40 replacements; Windows locked-file test verifies failure reporting and temp cleanup. At 49,500 CSS units, median write overhead 8.93 ms, worst 25.54 ms across those 40 writes. This excludes rendering and is not the full build-time measurement. |
| Favicon 100% while music plays | Fake clock: 3,001/3,001 audible samples through 300 s, plus every continuous advancing interval, including the first audible instant. The same interval checks cover an in-range seek, track change, same-track loop and pause/resume. Held silence is excluded. Live overlay acceptance remains for integration. |
| A/V offset <=0.25 s for >=95% | Every steady publication satisfies abs(publish media time + 0.5 s load delay - encoded start) <=0.25 s. Gate tests verify release delay subtraction at 0, 0.1 and 0.8 s output latency; startup at the measured 0.1 s output latency aligns picture and music. Actual image/audio sync is unmeasured. |
| T3 call latency median <0.3 s | Unmeasured live, as requested. API tests use mock servers; no simulation result establishes the live latency bar. |
| Restart/failure/planning behavior | Sixteen fake-clock tests cover successful and failed publications, in-range seeks, track changes/loop/end, pause/resume/off/quit, busy-worker replacement, queued/finished cancellation, late and failed encodes, stale completions, missing grants/hold acknowledgments, six-second timeout, gate off, configurable lengths and delayed starts. |

The complete 250 ms write loop, rendering time, actual Chromium visibility/sync, AVIF quality,
click accuracy and card size belong to integration or other crates. No live measurement is
claimed, and no live theme, settings, project, thread or helper was changed. There are no
`todo!()` implementations left in dt-t3. Idle-thread creation is implemented from the installed
source contract but has only mock validation; encoder/API/theme-loop I/O for scheduler actions
belongs to the binary.
