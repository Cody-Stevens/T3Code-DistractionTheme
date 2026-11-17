# dt-render

`NativeAvifEncoder` is the default choice for integration. It links libavif 1.4.2 and libaom
3.15.3, built with MSVC/CMake and `AOM_TARGET_CPU=generic`. No assembler or system package was
installed. The source archives, provenance, licenses, and build details are in [native/README.md](native/README.md).
The small C shim and `src/native.rs` own the codec allocation boundary; ordinary rendering is safe Rust.

The crate implements short AVIF clips, the encode-ahead size search, fallback resizing/quality
ladder, exact-rate favicon clips and padded SVGs, glow pixels, edge colors, the card, complete
static/dynamic/idle CSS, and favicon deck stacking. There are no remaining renderer stubs.
`encode_clip_svg_with` accepts any encoder and selects q55/speed10 for fast clips or q60/speed8
otherwise. The original `encode_clip_svg` entry point decodes with its configured ffmpeg path
and encodes with the native backend.

CSS matches `python/theme_css.py` on the rebased main branch. The front deck uses opacity 1,
the back deck .01, and `favicon_allow_css` hides stale clip basenames without restarting images.
The isolated text-flow block includes the 8 px margin, crossing clips and text-shadow copies,
and the row containment override. Dynamic writes automatically include Python's diagnostic CSS
when `GLOWTUBE_MEASURE` or `GLOWTUBE_SHOW_LOADING` is nonempty; empty or missing variables leave
it out. Diagnostics consume the same CSS budget as the other layers. A covered write can supply
one still video frame while glow and values animate for their full frame count.

The ffmpeg implementations remain available for experiments. A persistent worker streams IVF
packets and Rust muxes them into a primary AVIF item plus an image-sequence track. Each clip
starts at a keyframe. At most two processes retain video/glow configurations; shutdown closes
stdin and reaps them, with whole-tree termination on timeout. Ahead mode uses a fresh process
to flush libaom's lookahead. Neither backend writes temporary media files.

Measurements below were taken offline on 2026-11-17, Windows/MSVC, Ryzen 9 9900X (12 cores / 24
threads), while the other rewrite owners were working. Inputs were eight generated 640x360
`testsrc2` frames at 24 fps. Codec measurements use four threads; the actual ahead sizing helper
uses two, and favicon clips use six. Pillow 12.0.0/libavif 1.3.0 is the reference. PSNR uses decoded
RGB pixels and the size-curve interpolation used by `python/tools/encoder_match.py`. These are
synthetic measurements, not a claim about all video content or CPU contention.

| Encoder / setting | Encode median / worst | Output bytes at setting | RGB PSNR | Edge animation |
| --- | ---: | ---: | ---: | --- |
| Pillow speed10 q35 | 34.1 / 35.7 ms | 17,328 | 27.994 dB | Yes |
| Native generic speed10 q35, PSNR tune | 71.2 / 75.5 ms | 19,742 | 28.283 dB | Yes; holds last frame |
| Native generic speed6 q35, PSNR tune | 825.1 / 984.6 ms | 13,810 | 29.110 dB | Yes; holds last frame |
| Persistent ffmpeg libaom realtime q35 | 24.7 / 28.4 ms warm | 13,602 after warmup | 27.268 dB | Yes |
| One-shot ffmpeg libaom realtime q35 | 62.3 / 65.1 ms | 16,428 | 27.736 dB | Yes |
| ffmpeg libaom good/CPU6 q35 | 312.0 / 322.7 ms | 7,751 | 28.258 dB | Yes |
| ffmpeg SVT-AV1 preset12 CRF50 | 53.9 / 60.7 ms | 14,406 | 27.259 dB | Yes |
| ffmpeg rav1e speed10 QP180 | 174.8 / 181.9 ms | 13,600 | 27.483 dB | Yes |
| Pure Rust rav1e 0.8.1 speed10 QP180, assembly disabled | 327.4 / 328.7 ms | 24,564 | 28.045 dB | Yes |

Native and ffmpeg rows use six samples per setting, except SVT/ffmpeg rav1e (five), Pillow
(three), and the pure Rust rav1e probe (three). The persistent q35 startup sample was 75.0 ms;
its warm median excludes that sample. The first Pillow q20 sample was 103.7 ms. The previously
suspected 250 ms ffmpeg startup cost did not recur with this streaming path: ordinary one-shot
encodes finish in about 60–75 ms. Starting `taskkill` on every successful encode did impose a
large extra cost; normal shutdown now uses EOF and waits for exit.

Uncalibrated persistent constant-quality mode grew from about 12 KB to 30 KB at q20 over six
clips. The retained experiment instead uses libavif's realtime CBR quantizer bounds (target
quantizer +/-4), CPU9 for fast mode, and full-range YUV. This stabilizes size, but still loses
quality at equal size. The native backend uses PSNR tuning to meet the specified comparison
metric; it preserves Pillow's quality/speed classes and avoids process startup entirely.

| At equal 18,000-byte output | RGB PSNR | Difference from Pillow speed10 |
| --- | ---: | ---: |
| Pillow speed10 | 28.036 dB | baseline |
| Native generic speed10 | 28.086 dB | +0.050 dB |
| Native generic speed6 | 29.266 dB | +1.230 dB |
| Pillow speed6 | 29.214 dB | +1.178 dB |
| One-shot ffmpeg libaom realtime | 27.828 dB | -0.208 dB |
| Persistent ffmpeg libaom realtime | 27.615 dB | -0.421 dB |
| ffmpeg libaom good/CPU6 | 28.908 dB | +0.872 dB |
| SVT-AV1 | 27.400 dB | -0.636 dB |
| ffmpeg rav1e | 27.672 dB | -0.364 dB |
| Pure Rust rav1e, assembly disabled | 27.528 dB | -0.508 dB |

At the reference 18 KB quality (28.036 dB), interpolation gives approximately 17.6 KB for native
speed10, 21.5 KB for one-shot ffmpeg, 23.3 KB for persistent ffmpeg, 35.1 KB for SVT, 26.5 KB
for ffmpeg rav1e, and 24.4 KB for assembly-disabled Rust rav1e. Native speed6 also exceeds the
reference speed6 curve by 0.052 dB at 18 KB. Across 12/18/24 KB, native speed10 exceeds the
speed10 reference by 0.111/0.050/0.095 dB. The generic build therefore meets these measurements
without NASM. Installing NASM could accelerate ahead encoding, but is not required for the
measured normal write budget.

The pure Rust probe disables all rav1e default and assembly features, enables only Rust threading,
and uses the same AVIF muxer. RGB-to-YUV conversion is excluded from its timing, giving it a
favorable lower bound. The probe reports its full quality curve through QP240; its QP80/130/180
samples took 334–413 ms in the first run. Its source and standalone manifest live under `tools/`;
its build and lockfile remain in ignored `rust/target`, keeping rav1e out of production dependencies.

| Applicable quality bar | Result and method |
| --- | --- |
| Complete theme build median <100 ms / worst <200 ms | 91.7 / 113.0 ms across 20 release builds after the CSS re-sync, including eight glow renders, edge colors, both AVIF layers, CSS, and a cached four-row card with a loading status |
| CSS <=49,500 characters, no backslashes | 46,251–47,593 characters in those builds; hard validation and overflow tests, including diagnostic budget accounting |
| Short clip animation | Edge 157.0.4341.60 headless screenshots show distinct frames over time; single-play native output holds its final frame |
| Favicon exact 24 fps timing | Cumulative ties-to-even 42/41/42 ms durations; native container tests total 1,000 ms for 24 frames and 46,000 ms for 1,104 frames |
| Padded SVG | Generated one-second fast clip has 24 frames and 4,300,007 bytes; nested AVIF animates in Edge |
| Card <=9,000 base64 characters | Four-row Segoe card with loading status: 7,764 characters, 552x624 pixels, 16-color palette; missing-font bitmap fallback: 3,216 characters |
| Marker RGB exact | Every original marker pixel survives palette reduction and lossless WebP decoding, checked with the independent pure-Rust decoder |
| CSS faithful to Python | 19 generated golden fixtures cover two geometries, the isolated text-flow block, sparse keyframes, paused/idle values, data URLs, animation, height, deck opacity, clip allowlists, and all diagnostic combinations |
| Glow / edge colors | Python-generated color and glow samples; fractional three-pass Gaussian blur matches Pillow's kernel and integer rounding |
| Overlay >=96%, A/V offset <=0.2 s, 28/28 clicks at two scales, <=1/400 false hits | Deferred to the integration owner: live helper, T3 state, and port 47873 were not changed |

Card rendering/quantization is cached by the caller; generating a new four-row card takes about
0.7 seconds and is outside `build_dynamic`'s input contract. Complex scene changes can require
several fallback ladder attempts and exceed the synthetic timing sample. Ahead encoding is
slower with generic libaom (roughly 0.8–1.1 seconds per tested clip before sizing probes), so
integration should measure its six-worker scheduling margin with real content.

One deliberate fallback change avoids repeated failed upward probes: after a synchronous clip
fits, the state retains that level. The ahead workers perform quality exploration. Python's
upward probe on every fallback write caused two synchronous encodes and a 166 ms median in the
same sample. The final CSS layers and budgets remain the same. Text-flow properties, keyframes,
and player-band selectors are isolated in `src/text_flow.css` and `css::text_flow_css()` for later
re-syncs. The templates and golden fixtures are regenerated from the current Python reference.

Run the mandatory checks from `rust/`, with `CARGO_BUILD_JOBS=4`:

```powershell
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

To repeat codec measurements, use a private Python environment with Pillow 12, NumPy, and
websocket-client, and ffmpeg/ffprobe on PATH. `tools/probe_avif.py` generates the input and Pillow
curves; `cargo run --release -p dt-render --example codec_measure -- <absolute-directory> native`
measures native encoding. Substitute `persistent` or `oneshot` for the other backends.
`tools/compare_ffmpeg.py`, `tools/rav1e_probe.ps1`, and `tools/matched_quality.py` compare the
alternatives. `--example build_measure` measures complete writes and produces the padded SVG
when `source.mp4` is present. `tools/check_animation.py <directory>` opens a private headless Edge
profile, samples real-time screenshots, records the browser version, then kills its whole process
tree. It starts no web server. All generated media, screenshots, and logs stay under `rust/target`.
`tools/css_goldens.py` regenerates the CSS fixtures and templates from the current Python reference.
