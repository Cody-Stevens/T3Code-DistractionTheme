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
- **Theme video.** Animated AVIF (libavif via Pillow) at up to 640x360. libavif speed 6 is ~2 dB
  better than speed 10 at equal size but takes 200-400 ms per clip, so six worker processes encode
  clips ~1.25 s ahead and each write takes the matching one, falling back to a speed-10 encode.
  SVT-AV1 via ffmpeg was measured at matched size: no real difference, more overhead.
- **Glow.** Pre-rendered per frame by the helper (128x72, frame shrunk into the centre and blurred,
  widened by bass, kick shockwave ring), sent as an animated AVIF and stretched by CSS. Rendering
  the light in the helper is far cheaper than per-region color variables.
- **Text and the player.** Text blocks use a scroll-driven `view()` timeline to slide across the
  player band instead of sitting under it. Markdown blocks only in assistant rows; user bubbles move
  as a whole (moving both double-shifts).
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
- **T3 abandons an in-flight favicon load when a theme update lands.** With 4 writes/s a favicon
  swap took 6-11 s; with no writes it takes ~0.3 s. Before each swap the helper sends one long theme
  write (2.5 s of content) and holds writes for 2.2 s, publishing the clip 0.45 s into that window.
- The server serves an external favicon only while the file exists. Clients may fetch late, so keep
  recent clips (30) and give every clip a unique name (run id + sequence).
- Clip lengths ramp 8 s, 24 s, 45 s after a start or seek, because the next clip's encode (~0.2x its
  length) must finish while the current one plays. Frame durations alternate 41/42 ms to average
  exactly 24 fps. While a favicon clip covers the player, the hidden theme video is encoded cheaply.
- Every client signed in to the T3 server downloads the clips (the favicon is server state).

## Rules

- Never commit downloaded videos, clips, screenshots, logs or anything from `~/.t3`.
- The T3 token lives in `~\.secrets\t3code-distraction-theme\t3-token` (owner-only ACL), issued with
  only `orchestration:read` and `orchestration:operate`. Never print it, log it or commit it.
- Don't credit an agent, a model or AI in commits, pull requests, docs or code.
- Measure changes against the live app (`python/tools/`) rather than assuming; several plausible
  explanations above turned out wrong when measured.
