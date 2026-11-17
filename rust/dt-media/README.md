# dt-media

Media acquisition, audio analysis, buffered video, and cpal/WASAPI output for the Rust player.
The implementation follows `expand`, `load`, `analyse`, `VideoReader`, `Player._callback`, and
`media_time` in [`python/glowtube.py`](../../python/glowtube.py). No entry points remain stubbed.

## Integration

Call `expand`, `load`, and `decode_audio` on loader workers. All tool arguments go directly to
executables without a shell. Tool failures contain an exit code and static context, without
captured diagnostics. `DownloadConfig` supplies executables and the cache directory; use the
existing Python `cache` directory to share `<video id>.mp4` files. `load` also reuses a resolved
`TrackInfo.path`, so evicting PCM does not force another download. Every yt-dlp call specifies
`--js-runtimes node`. Metadata title/artist precedence and the <=360p format selector match Python.
Single-file downloads are remuxed to MP4 when necessary so the cache has the expected extension.

Keep only current/next `LoadedTrack` PCM resident, as the orchestration contract requires. Source
replacement transfers old PCM out of the audio callback for disposal on the calling thread.
Controls wait for the next callback acknowledgement, with a two-second failure timeout; send them
from the player/control worker. The callback does not acquire a player/control mutex, allocate,
invoke tools, or perform HTTP requests. It uses preallocated `rtrb` queues and atomic clock fields.
The output stream lives on an owned device thread because platform stream handles need not be
`Send` or `Sync`; `CpalOutput` itself implements the shared `Clock` contract.

`VideoReader.get` never waits for a decoded frame. It returns an exact hit, the nearest older
frame, or `None` while a restart is loading. The pipe stops advancing when its decoded head is
more than 48 frames ahead of the requested position. Before each write, call
`reader.begin_batch(k0)?`, then read `k0..end` with `get`. Each insertion discards frames older
than `min(batch_start, wanted) - 48`. Retention follows the write's start even when it reads 60
frames ahead and the next write advances only six frames. With 60-frame writes, the buffer holds
up to about 156 frames / 103 MiB. Update `begin_batch` for every write (or every standalone frame)
so the retention boundary advances.

A request older than the oldest retained frame restarts ffmpeg, including loops near EOF.
Requests more than 48 frames beyond the decoded head also restart, except during the first
second after a restart when they are within 96 frames of its starting frame. That grace period
lets the initial batch finish requesting frames while ffmpeg starts decoding. Explicit seeks
always restart and reset the buffer, retention boundary and grace period. Stop/drop terminate
and reap the owned process and join its worker; Windows termination uses `taskkill /T /F`.

## Clock behavior

The callback records its starting sample index and `Instant`, and subtracts cpal's
`playback - callback` timestamp difference. Between callbacks, `media_time` interpolates elapsed
time, bounded by the samples actually submitted to the device. It cannot advance indefinitely
if callbacks stop. Pause freezes the audible position; resume continues from that sample. Seek
and source replacement reset the clock boundary, preventing old device latency from reporting
positions before the seek target. The final callback anchor stays valid while its remaining
samples drain, so EOF reaches the exact PCM duration even while later callbacks emit silence.
`ended` becomes true when that audible clock reaches the end. Volume defaults to 0.8 and accepts
0..=1.5, including Python's int16 truncation/wrapping behavior above full scale.

Sample indexing is exact at 48 kHz (20.833 microseconds per sample); control boundaries round
to sample positions. Timestamp interpolation and the backend's latency estimate limit practical
accuracy. `device_latency()` reports that estimate, which may exclude physical hardware delay.
This is the same kind of audible-position estimate as Python's callback anchor minus stream
latency, rather than a measurement of sound leaving a speaker.

The binary can hold music for the first favicon clip through `AudioOutput::hold()` and
`release_at(Instant)`. Install a hold before starting, seeking, or resuming playback. Held
callbacks emit silence, consume no PCM, and publish a frozen clock at the PCM cursor. Holds
survive source replacement, seeks and pause/resume; those operations still update the frozen
position normally. `gated()` lets the write loop choose a still frame while the deadline is
pending. A new `hold()` cancels a previous scheduled release; `release_at` can replace a deadline,
and past deadlines release on acknowledgement.

Release allows audio on the first device callback at or after the monotonic deadline. The clock
stays frozen until that callback submits PCM, then resumes its latency-adjusted interpolation.
The deadline does not unpause playback. For a desired audible start time, the binary can subtract
`device_latency()` from that time before scheduling release, as Python does. The binary owns the
favicon publication decision and maximum hold timeout; dt-media applies neither automatically.

## Validation and measurements

Measured on Windows on 2026-11-16 with an AMD Ryzen 9 9900X, Rust 1.98.0 (MSVC), and
ffmpeg 9.0.2 (Gyan full build). Commands below run from `rust/`. Set `CARGO_BUILD_JOBS=4`
for every Cargo invocation and leave `CARGO_TARGET_DIR` unset. Normal tests require no external
media tools or audio device. Windows tool mocks use private temporary directories and fake
metadata/PCM, without network access, credentials, or T3 state.

The committed [analysis oracle](tests/analysis_golden.json) contains every feature for four seconds
of synthetic stereo sine sweeps and decaying bass bursts generated with ffmpeg lavfi. The
[generator](tests/generate_golden.py) extracts and runs the unmodified Python `analyse` AST with
NumPy 2.2.6, without importing the helper. The Rust test reconstructs identical s16le PCM and
checks its byte hash before comparing all 96 frames / 16 bands plus bass, kick, and ring. Its
absolute envelope tolerance is 0.0002; the measured maximum error was 0.000000886. Every ring
radius matched exactly, including the kick-triggered resets. Empty, sub-frame, and silent inputs
have separate coverage (Python raises on empty input; the Rust contract requires safe empty arrays).

The 2026-11-17 follow-up replays 13 writes / 780 frame requests: an initial 60-frame write,
six advances of six frames, then six advances of 48 frames, each reading 60 frames. The synthetic
pipe retained every write's first frame with zero restarts after the initial start. Separate
tests cover both restart-grace boundaries and backward seeks. Fake-output tests check indefinite
holds, exact scheduled deadlines, frozen clocks without callbacks, source replacement, seeks,
pause/resume, rescheduling, cancelled deadlines, past deadlines and latency after release.
These regressions require no audio device, external tools or T3 state.

```powershell
python dt-media/tests/generate_golden.py
cargo test --locked -p dt-media --test golden -- --nocapture
cargo test --locked -p dt-media --test video_ffmpeg -- --ignored --nocapture
cargo run --locked --release -p dt-media --example measure -- bench
cargo run --locked --release -p dt-media --example measure -- clock
```

The opt-in video test creates a seven-second FFV1 test pattern in a temporary directory. It
checks RGB24 dimensions, backward loops beyond retained history, explicit seeks, EOF loops,
far-ahead restarts, and idempotent shutdown. The in-memory pipe test verifies the exact 48-frame
history and 49-frame look-ahead boundary. Tool tests cover playlist order/null entries, metadata
precedence, ID-keyed downloads, cache reloads, local paths, unsafe IDs, malformed JSON, partial
PCM, and nonzero subprocess exits. Fake output tests cover latency subtraction, interpolation,
pause, seek, silence, partial EOF blocks, final-clock draining, gain, and deferred PCM disposal.

The four-minute benchmark generates 48 kHz stereo AAC and excludes fixture generation/build time.
Three release runs decoded 11,520,000 sample frames and analysed 5,760 feature frames:

| Run | Decode | Analysis | Total |
| --- | ---: | ---: | ---: |
| 1 | 203.205 ms | 112.106 ms | 315.311 ms |
| 2 | 173.148 ms | 109.314 ms | 282.462 ms |
| 3 | 176.453 ms | 109.424 ms | 285.877 ms |

The default-device clock measurement played a 220 Hz sine at approximately 1% full scale on
`Speakers (7- USB Audio CODEC )`. After a one-second warm-up, it sampled the clock 1,923 times
over five seconds against `Instant`, using each read's wall-time midpoint. After fitting a line,
residual jitter was 0.184804 ms RMS, 0.617653 ms at the 95th absolute percentile, and 0.736033 ms
peak to peak. The fitted rate differed from wall time by -29.432 ppm; no reads went backward.
Reported device latency was 10.000 ms. Live pause and seek assertions also passed, and the stream
was stopped before exit. This measures reported-clock stability, not physical speaker latency.

The crate satisfies the media-specific numerical, decode-time, buffer, and clock requirements.
Theme build/write budgets, AVIF quality/animation, overlay coverage, click accuracy, card size,
and live T3 A/V offset require other crates and integration testing; this crate does not claim
those results. The live Python helper, port 47873, T3 files, projects, and threads were untouched.
