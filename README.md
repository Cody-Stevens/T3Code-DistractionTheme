# T3Code-DistractionTheme

A deliberately over-the-top theme for [T3 Code](https://github.com/pingdotgg/t3code): a YouTube music
video plays in the middle of your chat, its light spills across the whole app, and everything
pulses with the bass. It is a joke about how much you can get away with inside a "CSS-only"
theme. It is not meant for getting work done.

- The video sits in a band across the chat; text slides across it as you scroll, so nothing stays
  hidden behind it.
- A glow drawn from the video's own colors fills the chat, and chat text acts as a mask for it.
- The glow widens and flashes on bass hits; the sidebar's thread rows become a 16-band spectrum.
- A Now Playing card under the thread details has play/pause, a scrubbable progress bar, an Up Next
  queue and "Play / Queue copied link" buttons.

## How it works

T3 Code themes are a JSON file with colors and up to 50,000 characters of CSS: no JavaScript, no
network requests, `url()` only for `data:` URLs. Everything here works inside those rules, with a
helper process doing the heavy lifting outside the app.

- **A theme that rewrites itself.** T3 watches its themes folder and pushes changes to the app. The
  helper rewrites the theme four times a second (writes faster than T3's 100 ms debounce never
  land). Each write carries the next 8 frames of video as an animated AVIF, a pre-rendered glow
  image, keyframes for the audio analysis, and the Now Playing card as a lossless WebP.
- **Full-quality video through a project icon.** A "GlowTube" project's favicon is pointed at
  8-45 second clips (animated AVIF wrapped in SVG, padded past 4 MiB so T3's icon cache skips them).
  T3 shows that favicon with a real `<img>`; the theme lifts it out of its sidebar row and anchors it
  over the player. This carries native 640x360 video with no 50 KB limit. The theme's own video
  runs underneath as a fallback.
- **Audio-driven everything.** The helper plays the audio itself and analyses it per video frame
  (16 log-spaced bands, bass envelope, kick detection), then sends those as CSS keyframes.
- **Clickable without JavaScript.** CSS can't receive clicks, so the card's controls are drawn in
  exact marker colors. The helper watches for left clicks inside T3 and reads a thin strip of pixels
  under the cursor (only inside T3's right-hand panel, discarded immediately) to tell which control
  was pressed.

Details and the constraints we ran into are in [AGENTS.md](AGENTS.md).

## Requirements

- Windows 10 or 11 (input handling uses Win32), T3 Code desktop
- Python 3.12+, `ffmpeg` and `yt-dlp` on `PATH`, Node.js (used by yt-dlp for YouTube)

## Setup

```powershell
cd python
python -m venv .venv
.venv\Scripts\python -m pip install -r requirements.lock
.venv\Scripts\python service.py setup   # T3 token, GlowTube project, theme
.venv\Scripts\python glowtube.py play "https://www.youtube.com/watch?v=..."
```

`service.py` (without `setup`) runs the player whenever T3 Code is running and restarts it if it
crashes. A Rust rewrite of the helper is in progress in `rust/`.

The T3 token is issued by `t3 auth session issue` with only `orchestration:read` and
`orchestration:operate`, and is stored outside the repository in
`~\.secrets\t3code-distraction-theme\t3-token`, readable only by your user.

## Controls

- On the card: play/pause, click the progress bar to seek, click an Up Next row to play it next,
  "Play copied link" / "Queue copied link" (copy a YouTube link first; playlists work).
- Keyboard media keys: play/pause, next, previous.
- Command line: `glowtube.py ctl toggle | next | prev | seek 10 | vol 0.8 | chaos 0-11 | add <url> | queue <url> | status | quit`

To go back to a normal theme, pick another theme in T3's appearance settings.
