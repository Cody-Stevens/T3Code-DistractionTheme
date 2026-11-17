# T3Code-DistractionTheme

A deliberately over-the-top theme for [T3 Code](https://github.com/pingdotgg/t3code): a YouTube music
video plays in the middle of your chat, its light spills across the whole app, and everything
pulses with the bass. It is a joke about how much you can get away with inside a "CSS-only"
theme. It is not meant for getting work done.

- The video sits in a band across the chat. Text hops over it as you scroll, coming within a few
  pixels of it, so nothing is hidden behind it.
- A glow drawn from the video's own colors fills the chat, and chat text acts as a mask for it.
- The glow widens and flashes on bass hits; the sidebar's thread rows become a 16-band spectrum.
- A Now Playing card under the thread details has play/pause, a scrubbable progress bar, an Up Next
  queue and "Play / Queue copied link" buttons.

## How it works

T3 Code themes are a JSON file with colors and up to 50,000 characters of CSS: no JavaScript, no
network requests, `url()` only for `data:` URLs. Everything here works inside those rules, with a
helper process doing the heavy lifting outside the app.

- **A theme that rewrites itself.** T3 watches its themes folder and pushes changes to the app. The
  helper rewrites the theme on a grid timed by the audio clock. Each write is complete on its own:
  frames of video as an animated AVIF, a pre-rendered glow image, keyframes for the audio
  analysis, and the Now Playing card as a lossless WebP.
- **Full-quality video through a project icon.** Two projects, "GlowTube" and "GlowTube B", take
  turns showing clips of up to 22 seconds as their favicon. Each clip is an animated AVIF wrapped in
  SVG and padded past 4 MiB, so T3's icon cache skips it. T3 shows a favicon with a real `<img>`.
  The theme lifts that image out of its sidebar row and anchors it over the player, which carries
  native 640x360 video with no 50 KB limit. While a clip covers the player, the theme is written
  only every 2 seconds, which keeps T3 responsive. The music waits for the first clip after a start
  or seek, so picture and sound stay in step.
- **Audio-driven everything.** The helper plays the audio itself and analyses it per video frame
  (16 log-spaced bands, bass envelope, kick detection), then sends those as CSS keyframes.
- **Clickable without JavaScript.** CSS can't receive clicks, so the card's controls are drawn in
  exact marker colors. The helper watches for left clicks inside T3 and reads a thin strip of pixels
  under the cursor (only inside T3's right-hand panel, discarded immediately) to tell which control
  was pressed.

Details, and the T3 behaviours all of this depends on, are in [AGENTS.md](AGENTS.md).

## Requirements

- Windows 10 or 11 and T3 Code desktop.
- `ffmpeg`, `ffprobe` and `yt-dlp` on `PATH`, and Node.js (yt-dlp uses it for YouTube).
- To build: Rust 1.88 or newer with the MSVC toolchain, and CMake. libavif and libaom are vendored
  and built automatically; no assembler is needed.

## Install

Start T3 Code, then:

```powershell
cd rust
cargo build --locked --release -p distraction
target\release\distraction.exe install
```

`install` does the following:

1. Issues a T3 access token for the helper, with only `orchestration:read` and
   `orchestration:operate`. The token is stored in `~\.secrets\t3code-distraction-theme\t3-token`,
   readable only by your user.
2. Creates the two pinned GlowTube projects (rooted in `%LOCALAPPDATA%\GlowTube\workspace`) and an
   idle thread for each, without sending any agent a message.
3. Writes the GlowTube theme and selects it.
4. Copies `distraction.exe` and `distraction-service.exe` to `%LOCALAPPDATA%\GlowTube\bin`.
5. Starts the windowless service now and at every sign-in (a per-user `Run` entry).

The service runs the player whenever T3 Code is running with the GlowTube theme selected, and
restarts it if it crashes, backing off on repeated crashes. Nothing plays until you give it a link.

**To turn it off**, pick another theme in T3's appearance settings; the service stops the player
within a few seconds. Pick GlowTube again to bring it back. **To remove it**, run
`distraction uninstall`. That stops the service and removes the sign-in entry. Then delete
`%LOCALAPPDATA%\GlowTube` if you want the files gone too. The T3 projects and token stay until you
remove them.

Logs are in `%LOCALAPPDATA%\GlowTube\service.log` and `player.log`. Set `GLOWTUBE_DEBUG=1` for
detailed timing logs.

## Controls

- On the card: play/pause, click the progress bar to seek, click an Up Next row to play it next,
  "Play copied link" / "Queue copied link" (copy a YouTube link first; playlists work).
- Keyboard media keys: play/pause, next, previous.
- Command line: `distraction ctl toggle | play | pause | next | prev | seek 10 | vol 0.8 | chaos 0-11 | add <url> | queue <url> | playnext <url> | status | quit`.
- Without the service: `distraction play <url>` runs the player in the terminal.

## Repository

- `rust/`: the helper. One crate per area (`dt-media`, `dt-render`, `dt-t3`, `dt-platform`) plus
  the `distraction` binary; see [docs/RUST_PLAN.md](docs/RUST_PLAN.md).
- `python/`: the original helper. It is the reference implementation and still runs
  (`python/service.py setup`, then `python/glowtube.py play <url>`). Its `tools/` folder has the
  measurement scripts used to tune both versions against the live app (favicon coverage,
  audio/video sync, T3 latency, write rate, click detection). Run only one helper at a time; both
  use port 47873.
