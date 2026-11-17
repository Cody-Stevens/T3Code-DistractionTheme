"""CSS for the GlowTube theme.

The helper sends pictures, not numbers, wherever it can:
  --vid   an animated AVIF of the next second of video, played by the browser itself
  --glow  an animated AVIF of the light the video throws: blurred, widened by the bass and
          carrying the kick shockwave, pre-rendered by the helper at 128x72 and stretched by CSS

Keyframes animate a few numbers for things a picture cannot reach:
  --b0..--b15  16-band spectrum, shared by the sidebar rows and the card's bars
  --bs bass, --kk bass kick, --pr progress
  --c0..--c3   top, right, bottom and left edge colors, --aa average color

Speed: the properties are registered with `inherits: false` and animated only on the elements that
read them (ANIMATED). Animating inherited properties on :root made the browser restyle the whole
app on every frame. Chat text no longer has animated shadows either; its pulse comes from the
glow image under the multiply layer, which the GPU composites.

T3 Code drops any CSS value that contains a backslash, so nothing here may use one.
"""

import card_image

BANDS = 16
COLOR_VARS = ["c0", "c1", "c2", "c3", "aa"]
NUMBER_VARS = [f"b{i}" for i in range(BANDS)] + ["bs", "kk", "pr"]
GLOW_SCALE = 4          # the glow image covers 4x the player in each direction

CONV = '[data-workspace-area="conversation"]'
CARD = "[data-thread-details-card]"
# Every element that reads an animated value. The keyframes run on each of these, nowhere else.
ANIMATED = (
    f'{CONV},{CONV}::before,{CONV}::after,[data-slot="composer-host"],[data-chat-header],'
    f'[data-slot="sidebar-inner"],[data-thread-item],{CARD},{CARD}::before,{CARD}::after'
)
# Spectrum color for band i, shared by the sidebar rows and the card's bars.
BAND_COLOR = [["c0", "c1", "c2", "c3"][i * 4 // BANDS] for i in range(BANDS)]


def _properties() -> str:
    out = [f"@property --{v}{{syntax:'<color>';inherits:false;initial-value:#000}}" for v in COLOR_VARS]
    out += [f"@property --{v}{{syntax:'<number>';inherits:false;initial-value:0}}" for v in NUMBER_VARS]
    return "".join(out)


def static_css(video_w: int, video_h: int) -> str:
    # The card's live layer: 16 spectrum bars and the progress bar, drawn over the helper's card image
    # into the gaps it leaves (see card_image.py for the layout, in CSS pixels).
    ci = card_image
    inner = ci.WIDTH - 2 * ci.PAD
    step = inner / BANDS
    bars = ",".join(
        f"linear-gradient(var(--{c}),color-mix(in srgb,var(--{c}) 25%,transparent))" for c in BAND_COLOR
    )
    span = ci.SPECTRUM_BOTTOM - ci.SPECTRUM_TOP - 2
    bar_sizes = ",".join(f"{step - 3:.2f}px calc(3px + var(--b{i})*{span}px)" for i in range(BANDS))
    bar_pos = ",".join(f"{ci.PAD + i * step:.2f}px calc({ci.SPECTRUM_BOTTOM - 3}px - var(--b{i})*{span}px)"
                       for i in range(BANDS))
    fill, track = (card_image.hexcolor(c) for c in (ci.MARK_FILL, ci.MARK_TRACK))
    progress = f"linear-gradient(90deg,{fill} calc(var(--pr)*100%),{track} 0)"
    bar_w = ci.WIDTH - ci.PAD - ci.BAR_LEFT
    bar_top = ci.CONTROL_Y - ci.BAR_HEIGHT // 2
    sidebar_rows = "".join(
        f"[data-thread-item]:nth-child({BANDS}n+{i + 1}){{background:linear-gradient(90deg,"
        f"color-mix(in srgb,var(--{BAND_COLOR[i]}) 50%,transparent) calc(var(--b{i})*100%),"
        f"transparent 0)!important}}"
        for i in range(BANDS)
    )
    glow = f"var(--glow) center/calc(var(--vw)*{GLOW_SCALE}) calc(var(--vh)*{GLOW_SCALE}) no-repeat"
    jumpers = (
        '.messages-timeline-scroll [data-message-role="assistant"] .chat-markdown > :not(ul,ol),'
        '.messages-timeline-scroll [data-message-role="assistant"] .chat-markdown > :is(ul,ol) > li,'
        ".messages-timeline-scroll [data-user-message-bubble],"
        '.messages-timeline-scroll [data-timeline-row-kind]:not([data-timeline-row-kind="message"])'
    )
    css = f"""{_properties()}
:root{{--g:1;--vw:{video_w}px;--vh:{video_h}px;--by:40%;--band:calc(var(--vh) + 56px)}}
{CONV},{CONV}::before{{--glow:linear-gradient(#0000,#0000)}}
{CARD}::before{{--cardimg:linear-gradient(#0000,#0000)}}
{CONV}{{isolation:isolate;overflow:hidden;
background:linear-gradient(color-mix(in srgb,#07060b calc(62% - var(--bs)*45%*var(--g)),transparent),color-mix(in srgb,#07060b calc(62% - var(--bs)*45%*var(--g)),transparent)),{glow},#07060b!important}}
.messages-timeline-scroll,.messages-timeline-scroll [data-timeline-row-kind],.messages-timeline-scroll [data-timeline-row-kind] *{{background-color:transparent!important}}
{CONV}::before{{content:"";position:absolute;inset:0;z-index:40;pointer-events:none;mix-blend-mode:multiply;
background:{glow},linear-gradient(color-mix(in srgb,var(--aa) 25%,#7a7a7a),color-mix(in srgb,var(--aa) 25%,#7a7a7a));
background-blend-mode:screen;will-change:transform}}
{CONV}::after{{content:"";position:absolute;z-index:41;pointer-events:none;left:50%;top:var(--by);
width:var(--vw);height:var(--vh);transform:translate(-50%,-50%);border-radius:12px;
background:var(--vid) center/cover no-repeat,#000;
box-shadow:0 0 0 1px #ffffff40,0 0 calc(18px + var(--bs)*70px*var(--g)) calc(var(--kk)*30px*var(--g)) var(--aa)}}
{CONV}{{anchor-name:--gtconv}}
/* Favicon video: the GlowTube project's icon is an animated clip (see glowtube.py FaviconVideo). Its
   sidebar row would contain and clip a fixed-position child (content-visibility, and the sidebar's
   scroll-fade mask), so both are lifted while the clip is present; then the image is pinned over the
   player with anchor positioning. */
[data-thread-item]:has(img[src*="glowtube-clip-"]){{content-visibility:visible!important;contain:none!important}}
[data-app-sidebar] [data-slot="scroll-area-viewport"]:has(img[src*="glowtube-clip-"]){{-webkit-mask:none!important;mask:none!important}}
img[src*="glowtube-clip-"]{{position:fixed!important;position-anchor:--gtconv;left:anchor(50%,900px)!important;top:anchor(40%,400px)!important;
translate:-50% -50%;width:var(--vw)!important;height:var(--vh)!important;max-width:none!important;max-height:none!important;
z-index:2147483000;border-radius:12px;object-fit:cover;pointer-events:none}}
@keyframes gtjump{{from{{translate:0 0}}to{{translate:0 calc(var(--band)*-1)}}}}
{jumpers}{{animation:gtjump linear both;
animation-timeline:view(block 0px calc(100% - var(--by) - var(--band)/2));
animation-range:entry-crossing 0% entry-crossing 100%}}
.messages-timeline-scroll{{padding-top:var(--band)!important}}
{CONV} :is(.chat-markdown,[data-user-message-bubble],[data-timeline-row-kind]),{CONV} :is(.chat-markdown,[data-user-message-bubble],[data-timeline-row-kind]) *{{
color:#fff!important;text-shadow:0 0 3px #ffffffa0}}
[data-slot="composer-host"]{{box-shadow:0 0 0 1px var(--c2),0 0 calc(12px + var(--bs)*60px*var(--g)) var(--c2),0 0 calc(var(--kk)*100px*var(--g)) var(--c2)!important}}
[data-chat-header]{{box-shadow:0 1px calc(4px + var(--bs)*40px*var(--g)) var(--c0)}}
[data-slot="sidebar-inner"]{{box-shadow:inset -1px 0 0 var(--c3),inset -14px 0 calc(var(--bs)*70px*var(--g)) -10px var(--c3)}}
{sidebar_rows}
::-webkit-scrollbar-thumb{{background:linear-gradient(#ff2bd6,#18d6ff)!important;border-radius:8px}}
{CARD}{{contain:none!important;position:relative!important;overflow:visible!important;
margin-bottom:calc(var(--cardh,240px) + 24px)!important;
box-shadow:0 0 calc(var(--bs)*50px*var(--g)) color-mix(in srgb,var(--c1) 60%,transparent)}}
{CARD}::before,{CARD}::after{{content:"";position:absolute;left:0;top:calc(100% + 12px);width:{ci.WIDTH}px;height:var(--cardh,240px);border-radius:14px}}
{CARD}::before{{z-index:5;background:var(--cardimg) 0 0/{ci.WIDTH}px auto no-repeat,#110a1d;
box-shadow:0 0 0 1px color-mix(in srgb,var(--aa) 70%,#fff),0 0 calc(14px + var(--bs)*60px*var(--g)) var(--c1)}}
{CARD}::after{{z-index:6;cursor:pointer;background-repeat:no-repeat;
background-image:{bars},{progress};
background-size:{bar_sizes},{bar_w}px {ci.BAR_HEIGHT}px;
background-position:{bar_pos},{ci.BAR_LEFT}px {bar_top}px}}
"""
    return css


def card_height_css(height: int) -> str:
    return f"{CARD}{{--cardh:{height}px}}"


# Every write must carry everything: T3 restarts all CSS transitions and animations when it applies
# a theme update, so nothing can be held on screen from an earlier write (tested in T3 with probes).
def card_image_css(card: "card_image.Card") -> str:
    return f'{CARD}::before{{--cardimg:url("data:image/webp;base64,{card.image}")}}'


def glow_css(data: str) -> str:
    return f'{CONV},{CONV}::before{{--glow:url("data:image/avif;base64,{data}")}}'


def keyframe_block(name: str, frames: list[dict]) -> str:
    """Keyframes for per-frame values. A frame may leave a property out; the browser blends across."""
    n = len(frames)
    parts = []
    for i, values in enumerate(frames):
        if not values:
            continue
        pct = 0 if n == 1 else i * 100 / (n - 1)
        body = ";".join(f"--{k}:{v}" for k, v in values.items())
        parts.append(f"{pct:.2f}%{{{body}}}")
    return f"@keyframes {name}{{{''.join(parts)}}}"


def animate(name: str, seconds: float, iterate: str = "forwards") -> str:
    return f"{ANIMATED}{{animation:{name} {seconds:.3f}s linear {iterate}}}"


def static_values(values: dict) -> str:
    return ANIMATED + "{" + ";".join(f"--{k}:{v}" for k, v in values.items()) + "}"


def idle_css() -> str:
    """No video: dark player, a soft glow, edge colors drifting through a palette."""
    hues = ["#ff2bd6", "#7a3cff", "#18d6ff", "#2bff88", "#ffe14d", "#ff7a1a", "#ff2b4a", "#c03cff"]
    frames = []
    for step in range(9):
        values = {f"c{i}": hues[(i * 2 + step) % 8] for i in range(4)}
        values["aa"] = hues[step % 8]
        values.update({f"b{i}": round(0.15 + 0.12 * ((i + step) % 4), 2) for i in range(BANDS)})
        values.update(bs=0.2, kk=0, pr=0)
        frames.append(values)
    return (keyframe_block("gtidle", frames) + animate("gtidle", 16, "infinite")
            + ":root{--vid:linear-gradient(#000,#000)}"
            + f"{CONV},{CONV}::before{{--glow:radial-gradient(closest-side,#7a3cff,#ff2bd640 40%,transparent)}}")
