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

Feed `FaviconScheduler::tick` loaded track metadata, audible media time, monotonic wall time,
encode completions and the latest quiet-write grant. Execute returned actions in order:

- Queue `ClipJob` on one dedicated serial encoder. The job carries deck, start, duration,
  output, quality/speed intent and estimated completion time. Return its actual frame count.
- For `RequestQuiet`, build 2.5 seconds of cheap theme content, land the write and hold further
  writes for 2.2 seconds. Pass the **time the write landed** in `quiet_granted_at`; this crate
  adds the 450 ms settle. Python stores the already-settled time instead.
- Publish the supplied path to the supplied project. Apply `SetFront` through the renderer's
  `favicon_front_css(deck.name())` in each complete write. `SetLive` controls cheap hidden video.
- Write `BLANK_SVG` to `clips_dir/glowtube-blank.svg` before executing `Blank`. Retain
  the newest 30 owned `glowtube-clip-*.svg` files on `Cleanup`; do not remove current files early.
- Cancel jobs on `CancelPending`; late completions are rejected by exact job identity. Cancellation
  need not interrupt a running encoder. Call `stop` and execute its blanks before shutdown.

Pass distinct project IDs, an absolute clip directory and a unique run ID (a UUID is suitable).
Each filename includes deck, run and sequence. Deck B is optional for Python's single-deck
fallback, which exposes theme video while swapping. The API changes are confined to dt-t3;
no shared or sibling crate changed.

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

## Reference parity and retained flaws

The current Python methods plan full durations 6.0, 12.167832, 31.006406 and 48.0 seconds for
the default 500 ms show latency. The last is 45 seconds plus its 3-second handover. Starts are
3.0, 6.6, 15.767832 and 43.774238 seconds. These are adaptive encode-budget results; the older
8/24/45 ramp in the workspace README is obsolete.

Python marks a deck's clip live even if its favicon API request fails: `_publish` catches the
error, then `_tick` assigns `deck.clip` unconditionally. Rust retains this optimistic state for
`PublishFailed`, so successful simulation coverage does not cover API failures. Python also
detects seeks by comparing media time with the front clip's range and pending deadline. A seek
that stays within that range is not detected and can leave the SVG animation out of sync. The
Rust simulation records that limitation rather than silently changing reference behavior.

The first publication also marks the deck live before the image loads. A cold long-write build
of 1.6 seconds delays first publication to about 3.4 seconds; theme video covers the initial gap.
Track changes and seeks outside the front range blank both decks and rebuild the short ramp.

## Validation

Run from `rust/`, with `CARGO_BUILD_JOBS=4` and the default worktree `target`:

```text
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked -p dt-t3 -- --nocapture
python dt-t3/tests/reference_replay.py
```

The optional Python replay extracts only constants and scheduler definitions from the reference
AST. It replaces clocks, encoders, API calls and cleanup; it never imports or starts the helper.
Rust tests have no Python dependency. Its first starts, durations, publications and flips are
golden assertions in the Rust simulation.

Measured on this Windows worktree with debug tests:

| Applicable quality bar | Result and method |
| --- | --- |
| CSS <=49,500, name <=48 | Boundary tests accept the limit, reject one unit over it, and check astral Unicode as two UTF-16 units. |
| Atomic theme writes | Concurrent reader sees complete JSON through 40 replacements; Windows locked-file test verifies failure reporting and temp cleanup. At 49,500 CSS units, median write overhead 14.41 ms, worst 35.84 ms across those 40 writes. This excludes rendering and is not the full build-time measurement. |
| Favicon >=96% visible | Fake clock: 2,971/2,971 covered samples from 3.0 to 300.0 seconds, plus continuous front-interval coverage assertions between every flip. This is 100% modeled steady coverage; live overlay acceptance remains for integration. |
| A/V offset <=0.2 s | Simulation asserts abs(publish media time + 0.5 s load delay - encoded start) <=0.2 s for every steady publication, and waits >=1.2 s to raise back decks. Live image/audio sync is unmeasured. |
| Late/start/seek/end behavior | Cold and slow-cold starts, two seek directions, track change/end, late and failed encodes, stale completions, pause/disable/stop and missing quiet grants all use fake clocks without sleeps. |

The complete 250 ms write loop, rendering time, actual Chromium visibility/sync, AVIF quality,
click accuracy and card size belong to integration or other crates. No live measurement is
claimed, and no live theme, settings, project, thread or helper was changed. There are no
`todo!()` implementations left in dt-t3. Idle-thread creation is implemented from the installed
source contract but has only mock validation; encoder/API/theme-loop I/O for scheduler actions
belongs to the binary.
