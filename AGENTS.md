# Working on T3Code-DistractionTheme

Read this before changing anything. Every constraint below was found by measuring the running app;
most of them are not documented by T3 Code.

## Layout

- `python/`: the working reference implementation.
  - `glowtube.py`: the helper. Player (audio via sounddevice, yt-dlp + ffmpeg decoding), per-frame
    audio analysis, theme writer, encode-ahead pool, favicon video, Win32 input (clicks, clipboard,
    media keys), local control server on 127.0.0.1:47873.
  - `theme_css.py`: all CSS. The static part (~10 KB) plus helpers for the per-write parts.
  - `card_image.py`: draws the Now Playing card (Pillow), defines the click marker colors.
  - `encoder.py`: worker-process encoders (short AVIF clips sized to fit, long favicon SVG clips).
  - `t3api.py`: T3 HTTP API client (bearer token, project mutations).
  - `service.py`: supervisor: runs the helper while T3 runs, restarts it, first-run setup.
  - `tools/`: measurement scripts used to tune all of the above (screen grabs of the live window,
    sync measurement, overlay visibility charts, encoder comparisons, click-detection tests).
- `rust/`: the Rust rewrite (in progress). It must match the Python behaviour; when they differ,
  the Python version and the measurements below are the specification.

## How T3 Code treats a theme (version 0.1.10 nightly, Electron 44 / Chromium 152)

- A theme is `~/.t3/userdata/themes/<id>.json`: `{version, name<=48, appearance, canvas, accent,
  colors, css<=50000}`. The server watches the folder with a **100 ms trailing debounce**, so writes
  closer than ~150 ms apart are merged. Replace the file atomically (write a temp file, `os.replace`).
- The client applies only the selected theme's CSS, with one adopted `CSSStyleSheet` and
  `replaceSync`, then sanitizes it: any declaration whose value contains a backslash is dropped;
  `url()` is allowed only as a quoted `data:` URL or `#id`; `image-set()`, `src()` and `@import` are
  removed. `@keyframes`, `@property`, `@container` and anchor positioning survive.
- Theme selection is `defaultTheme` in `~/.t3/userdata/settings.json` (watched too).
- **Nothing survives a theme write.** T3 adds a `no-transitions` class to `<html>` while applying a
  theme, so every CSS transition restarts; animation names must change each write so keyframes
  restart. Every write must be complete on its own.
- `colors` values are parsed as colors and only ~61 known roles are kept: no side channel there.

## The pieces and why they are the way they are

- **Write cadence.** 4 writes/s (250 ms), each with 8 frames (6 + 2 spare) at 24 fps. Writes are
  timed by the audio clock on a 6-frame grid so clips encoded ahead line up exactly.
- **Every theme write restyles the whole page, and that is the main cost to T3.** In a long thread,
  4 writes/s kept T3's renderer so busy that its own server calls waited ~2.2 s for a reply (median;
  2 ms with the theme idle; `tools/t3_latency.py` reads this from T3's trace log). That delayed every
  favicon load (below). So while a favicon clip covers the player (the theme's video is hidden),
  a write carries 2 s of glow, values and card (`COVERED_STEP` = 48 frames, +12 spare) with a single
  still video frame: one write per 2 s, ~0.14 s call latency. The text-flow rules are part of the
  cost: the current ones (text-shadow copies, three timelines per block) cost ~4x the simpler ones
  per write.
- **Frame reader.** Keep decoded frames from 2 s before the *start* of the latest write. Trimming
  from the last frame asked for discarded the next write's first frames, so every 2 s write restarted
  the decoder and came back empty.
- **Theme video.** Animated AVIF (libavif via Pillow) at up to 640x360. libavif speed 6 is ~2 dB
  better than speed 10 at equal size but takes 200-400 ms per clip, so six worker processes encode
  clips ~1.25 s ahead and each write takes the matching one, falling back to a speed-10 encode.
  SVT-AV1 via ffmpeg was measured at matched size: no real difference, more overhead.
- **Glow.** Pre-rendered per frame by the helper (128x72, frame shrunk into the centre and blurred,
  widened by bass, kick shockwave ring), sent as an animated AVIF and stretched by CSS. Rendering
  the light in the helper is far cheaper than per-region color variables.
- **Text and the player.** Text blocks use scroll-driven `view()` timelines to hop over the player
  band instead of sitting under it, coming within 8 px of it (`--gm`). A crossing block stays put
  under a clip-path while a text-shadow copy one band higher shows its lines that have crossed, then
  it moves up by one band (`_text_flow` in `theme_css.py`). T3 sets an inline
  `contain: paint layout style` on chat rows, which clipped hopped text; the theme drops the paint
  part. Markdown blocks only in assistant rows; user bubbles move as a whole (moving both
  double-shifts). Measure with `tools/gap_mock.py` / `gap_measure.py`.
- **Card.** CSS cannot lay out multi-line text without backslashes, so the card is a lossless WebP
  (16-color palette at 2x, ~8 KB) drawn by the helper, with the live spectrum and progress bar as
  CSS layers on top. Its clickable areas use exact marker colors (`card_image.MARKS`); clicks are
  accepted only in T3's right-hand panel and only when a solid area of one marker color is under the
  cursor (busy video frames otherwise produce false hits).
- **Performance.** Animated custom properties are registered with `inherits: false` and animated
  only on the elements that use them. Animating inherited properties on `:root` restyles the whole
  app every frame.

## Favicon video (full quality, beyond the 50 KB limit)

- A project's `faviconPath` may be an absolute path to an image file (an "external override").
  T3 renders project favicons as `<img src="/api/assets/<token>/<file name>">`, so CSS can match
  the file name: `img[src*="glowtube-clip-"]`.
- The client tries to cache favicons as data URLs up to 32,768 characters. A larger SVG makes its
  loader throw and fall back to the original asset URL, which keeps full resolution and animation.
  The loader reads up to 4 MiB first; **files over 4 MiB are refused immediately** (by content
  length), so clips are padded past 4.3 MB to skip a wasted full download.
- The image lives in the GlowTube thread's sidebar row. Two things trap a fixed-position child there:
  the row's `content-visibility: auto` and the sidebar scroll viewport's **scroll-fade mask**. The
  theme lifts both (only while the clip is present) and anchors the image over the player with
  `anchor-name` / `position-anchor`.
- **Favicon loads are slow while the theme is writing.** With 4 writes/s a favicon swap took 6-11 s;
  with no writes it takes ~0.3 s. Before each swap the helper sends one long theme
  write (2.5 s of content) and holds writes for 2.2 s, publishing the clip 0.45 s into that window.
- The server serves an external favicon only while the file exists. Clients may fetch late, so keep
  recent clips (30) and give every clip a unique name (run id + sequence).
- **How T3 swaps a favicon** (`ProjectFavicon.tsx` in T3's source): the visible `<img>` keeps the old
  file while a hidden `<img class="hidden">` loads the new URL; `onLoad` swaps them. A new URL means a
  new image, so **its animation starts from frame 0 when it is shown**. Each URL carries a token the
  client mints with `assets.createUrl`; when that call was slow (see the write cadence above), the
  client swapped the same clip a second time 7-10 s after publishing, restarting it 10+ s behind the
  audio. `tools/load_watch.py` (with `GLOWTUBE_SHOW_LOADING=1`) shows each deck's loader.
- **Two decks.** Two projects, "GlowTube" (a) and "GlowTube B" (b, rooted at `deck-b`), so each clip
  loads behind the one on screen. The next clip is published to the back deck 3 s before the front
  clip ends; it starts playing underneath in step with the audio, and 1.2 s later it is brought
  forward (`favicon_front_css`; clip names carry the deck letter,
  `glowtube-clip-<deck>-<run>-<seq>.svg`). **z-index does not order the two images**: GlowTube's
  row always painted over GlowTube B's whatever their z-index. So the back deck gets opacity 0.01
  (still painted, so it keeps playing) and the front deck opacity 1.
- **Stale clips are hidden by name, not replaced.** After a seek, publishing blank favicons to both
  decks collided with loading the first new clip (it never showed). Instead the theme lists the
  clip files allowed to show (each deck's current clip and the planned ones,
  `favicon_allow_css`) and hides every other clip image. Blanks are published only when the favicon
  video stops (pause, `favicon off`, quit).
- **Clip plan.** Clips are planned back to back from encode speed (speed 10 ~0.15 s and speed 8
  ~0.22 s per second of clip, +0.3 s start, 1.3x margin): each next clip is as long as the encoder
  can finish before it is due, up to 22 s (+3 s handover). Longer clips (45 s) went out of sync in
  the measurements. The first clip after a start, seek or resume is 10 s at the fast setting;
  publishing the next one only ~4 s after a 6 s first clip made the first blink out.
- **The music waits for the picture.** After a start, seek or resume the audio is held (silence, the
  clock frozen) until the first clip is published, then released ~0.5 s later, when it shows; at
  most 6 s (`GLOWTUBE_GATE=0` turns this off). A busy encoder (a 45 s clip takes ~10 s) is replaced
  rather than waited for. Measured: 100% coverage while music plays across start, seeks,
  pause/resume and the track looping; audio/video offset median 0.08 s, 95% within 0.25 s
  (`tools/sync_full.py`, which matches the screen against the whole source video).
- The long quiet write is built cheaply (a still video frame under a clip, ~0.25 s); request it
  ~1.75 s before a clip starts. Frame durations alternate 41/42 ms to average exactly 24 fps.
- Each favicon only renders inside a sidebar thread row, so each deck project needs one idle thread.
  `t3 thread start` always sends an agent message, so the Rust setup creates them through T3's HTTP
  MCP endpoint (`/mcp`, tool `t3_thread_launch` with no message), which starts no turn. That
  endpoint only accepts sessions issued with `--subject mcp-client`, and T3 refuses those sessions
  everywhere else, including the HTTP API the player uses. So the helper's long-lived token is a
  generic session, and setup issues a separate 15-minute `mcp-client` session (same two scopes),
  keeps it in memory, uses it for the two idle threads and revokes it at once. If that session
  can't be issued, setup still does the rest and reports that the threads must be made by hand.
- Measure coverage with `GLOWTUBE_MEASURE=1` (the theme paints the player magenta under the clips)
  and `tools/overlay_chart.py`.
- Every client signed in to the T3 server downloads the clips (the favicon is server state).

## Install and the service (Rust)

- `distraction install` runs setup (token, both deck projects rooted in
  `%LOCALAPPDATA%\GlowTube\workspace`, idle threads, theme file, theme selection), copies
  `distraction.exe` and the windowless `distraction-service.exe` to `%LOCALAPPDATA%\GlowTube\bin`,
  and registers the service in `HKCU\...\CurrentVersion\Run` as `GlowTube`.
- The service runs the player only while T3's server is alive **and** `settings.json` selects the
  `glowtube` theme. Picking another theme is the off switch. Because setup selects the theme, the
  service must not run setup while another theme is selected, or it would undo the user's choice
  on every T3 start.
- The player lives in a kill-on-close job owned by the service, so killing the service kills the
  player without blanking the favicons or writing the idle theme. Stop it with the stop-request
  file (`%LOCALAPPDATA%\GlowTube\service.stop`, which `distraction uninstall` writes); the service
  then quits the player through its control port and exits.
- The Rust service's mutex (`Local\GlowTubeDistractionService`) differs from the Python service's,
  so the two don't exclude each other. Run only one helper (both use port 47873).

## Rules

- Never commit downloaded videos, clips, screenshots, logs or anything from `~/.t3`.
- The T3 token lives in `~\.secrets\t3code-distraction-theme\t3-token` (owner-only ACL), issued with
  only `orchestration:read` and `orchestration:operate`. Never print it, log it or commit it.
- Don't credit an agent, a model or AI in commits, pull requests, docs or code.
- Measure changes against the live app (`python/tools/`) rather than assuming; several plausible
  explanations above turned out wrong when measured.
