"""Render the static theme CSS on a mock chat timeline in headless Edge, at several scroll positions.

  python tools/gap_mock.py <out-prefix> [scrollTop ...] [--h=<window height>]

Checks the text flow around the player: text should stop a small margin from the player's edges,
never be drawn under it, and leave no large empty band. One PNG per scroll position.
"""
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import theme_css  # noqa: E402

args = [a for a in sys.argv[1:] if not a.startswith("--")]
opts = dict(a[2:].split("=", 1) for a in sys.argv[1:] if a.startswith("--"))
prefix = Path(args[0])
tops = [int(a) for a in args[1:]] or [0, 400, 900, 1400]
height = int(opts.get("h", 1000))
css = theme_css.static_css(800, 450) + ":root{--vid:linear-gradient(#335,#224)}"

lorem = ("Text flows past the player and must stop just short of its edges. "
         "This sentence is long enough to wrap over several lines in the column. ")
rows = []
for i in range(14):
    rows.append(f'<div data-timeline-row-kind="message" data-message-role="user"><div data-user-message-bubble '
                f'style="margin:16px 0 16px auto;width:60%;border:1px solid #888;border-radius:12px;padding:8px 12px">'
                f'User bubble {i}: please tighten the gap.</div></div>')
    body = "".join(f"<p>Block {i}.{j}: {lorem * (1 + (i + j) % 3)}</p>" for j in range(3))
    body += f"<ul><li>List item {i}.a short</li><li>List item {i}.b {lorem}</li></ul>"
    if i % 5 == 2:
        body += "<pre style='margin:0'>" + "\n".join(f"code line {k}" for k in range(30)) + "</pre>"
    rows.append(f'<div data-timeline-row-kind="message" data-message-role="assistant"><div class="chat-markdown">{body}</div></div>')
    rows.append(f'<div data-timeline-row-kind="work" style="padding:6px 0">&#9656; Ran {i} commands and changed 1 file</div>')

html = f"""<!doctype html><html><head><style>
*,::before,::after{{box-sizing:border-box}}
html,body{{margin:0;height:100%;background:#07060b;font:15px/24px sans-serif;color:#ddd}}
.main{{position:absolute;left:280px;width:1200px;top:48px;height:{height - 48}px}}
.messages-timeline-scroll{{height:100%;overflow-y:auto}} .col{{max-width:720px;margin:0 auto;padding:0 16px}}
p{{margin:0 0 16px}} ul{{margin:0 0 16px}}
</style></head><body><div class=main><div data-workspace-area="conversation" style="position:relative;height:100%">
<div class="messages-timeline-scroll"><div class=col>{''.join(rows)}<div style="height:600px"></div></div></div></div></div>
<script>
const s = new CSSStyleSheet(); s.replaceSync({css!r}); document.adoptedStyleSheets = [s];
const sc = document.querySelector('.messages-timeline-scroll');
sc.scrollTop = Number(location.hash.slice(1)) || 0;
setTimeout(() => {{
  const out = [];
  for (const el of document.querySelectorAll('.chat-markdown > *, .chat-markdown li, [data-user-message-bubble], [data-timeline-row-kind="work"]')) {{
    const r = el.getBoundingClientRect(), cs = getComputedStyle(el);
    if (r.bottom < 0 || r.top > innerHeight) continue;
    const an = el.getAnimations().map(a => a.animationName + ':' + a.effect.getComputedTiming().progress?.toFixed?.(3) + ':' + a.effect.getComputedTiming().activeDuration);
    out.push([el.tagName, Math.round(r.top), Math.round(r.bottom), cs.getPropertyValue('--gtp'), cs.translate, an.join(','), cs.clipPath.slice(0, 160)].join(' | '));
  }}
  out.unshift('scrollTop ' + sc.scrollTop + ' sc ' + JSON.stringify(sc.getBoundingClientRect()));
  const pre = document.createElement('pre'); pre.id = 'dbg'; pre.textContent = out.join('\\n');
  pre.style.cssText = 'position:fixed;left:0;top:0;width:275px;margin:0;font:9px/11px monospace;color:#7f7;white-space:pre-wrap;z-index:99';
  if (location.search.includes('dbg')) document.body.append(pre);
}}, 500);
</script></body></html>"""
page = Path.home() / "AppData/Local/Temp/gt_gap_mock.html"
page.write_text(html, encoding="utf-8")
edge = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
prof = Path.home() / "AppData/Local/Temp/gt-edge-gapmock"
prefix.parent.mkdir(parents=True, exist_ok=True)
for top in tops:
    out = Path(f"{prefix}-{top}.png").resolve()
    subprocess.run([edge, "--headless=new", "--no-first-run", f"--user-data-dir={prof}", f"--window-size=1500,{height}",
                    "--virtual-time-budget=1500", f"--screenshot={out}",
                    page.as_uri() + ("?dbg" if "dbg" in opts else "") + f"#{top}"],
                   timeout=90, capture_output=True)
    print(out)
shutil.rmtree(prof, ignore_errors=True)
page.unlink(missing_ok=True)
