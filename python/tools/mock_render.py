"""Render theme CSS on a mock T3 layout in headless Edge and save a screenshot.

  python tools/mock_render.py <out.png> [theme.json ...]

Each theme file is applied in turn with replaceSync, 250 ms apart, the way T3 applies successive
writes; the screenshot is taken after the last one. Also prints which @property rules Chromium kept,
and the computed --glow / --cardimg values.
"""
import json
import shutil
import subprocess
import sys
from pathlib import Path

out = Path(sys.argv[1])
files = sys.argv[2:] or [str(Path.home() / ".t3/userdata/themes/glowtube.json")]
sheets = [json.load(open(f, encoding="utf-8"))["css"] for f in files]
html = f"""<!doctype html><html><head><style>
html,body{{margin:0;height:100%;background:#07060b;font:15px/1.5 sans-serif;color:#ddd}}
.app{{display:flex;height:100%}} .side{{width:260px}} .main{{flex:1;position:relative;display:flex;flex-direction:column}}
.scroll{{height:100%;overflow-y:auto}} .col{{max-width:760px;margin:0 auto;padding:0 16px}}
.panel{{position:absolute;right:16px;top:16px;width:276px}}
[data-thread-details-card]{{height:120px;background:#1b1530;border-radius:16px}}
</style></head><body><div class=app><div class=side data-slot=sidebar></div><div class=main>
<div data-workspace-area="conversation" style="position:relative;display:flex;flex-direction:column;flex:1;min-height:0">
<div class="messages-timeline-scroll scroll"><div class=col><div data-timeline-row-kind="message" data-message-role="assistant">
<div class="chat-markdown">{"".join(f"<p>Paragraph {i}. Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor.</p>" for i in range(30))}</div></div></div></div></div>
<div class=panel><div data-thread-details-card></div></div></div></div>
<pre id=log style="position:absolute;left:0;bottom:0;margin:0;color:#0f0;font-size:11px"></pre>
<script>
const sheets = {json.dumps(sheets)};
const s = new CSSStyleSheet(); document.adoptedStyleSheets = [s];
const steps = [];
const peek = () => {{
  const conv = document.querySelector('[data-workspace-area="conversation"]');
  const g = getComputedStyle(conv).getPropertyValue('--glow'), gb = getComputedStyle(conv, '::before').getPropertyValue('--glow');
  const c = getComputedStyle(document.querySelector('[data-thread-details-card]'), '::before').getPropertyValue('--cardimg');
  return (g.startsWith('url') ? 'G' : '.') + (gb.startsWith('url') ? 'g' : '.') + (c.startsWith('url') ? 'C' : '.');
}};
sheets.forEach((css, i) => setTimeout(() => {{ s.replaceSync(css); setTimeout(() => steps.push(i + ':' + peek()), 20); }}, i * 250));
setTimeout(() => {{
  const props = [...s.cssRules].filter(r => r instanceof CSSPropertyRule).map(r => r.name);
  const conv = document.querySelector('[data-workspace-area="conversation"]');
  const glow = getComputedStyle(conv).getPropertyValue('--glow').slice(0, 40);
  const card = getComputedStyle(document.querySelector('[data-thread-details-card]'), '::before').getPropertyValue('--cardimg').slice(0, 40);
  const bg = getComputedStyle(conv).backgroundImage.slice(0, 60);
  document.getElementById('log').textContent = JSON.stringify({{steps: steps.join(' '), props: props.length, glowRegistered: props.includes('--glow'), glow, card, bg}});
}}, sheets.length * 250 + 300);
</script></body></html>"""
page = Path.home() / "AppData/Local/Temp/gt_mock_render.html"
page.write_text(html, encoding="utf-8")
edge = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
prof = Path.home() / "AppData/Local/Temp/gt-edge-mock"
budget = str(len(sheets) * 250 + 1200)
subprocess.run([edge, "--headless=new", "--no-first-run", f"--user-data-dir={prof}", "--window-size=1500,900",
                f"--virtual-time-budget={budget}", f"--screenshot={out.resolve()}", page.as_uri()], timeout=90, capture_output=True)
dom = subprocess.run([edge, "--headless=new", "--no-first-run", f"--user-data-dir={prof}",
                      f"--virtual-time-budget={budget}", "--dump-dom", page.as_uri()], timeout=90, capture_output=True, text=True).stdout
shutil.rmtree(prof, ignore_errors=True)
page.unlink(missing_ok=True)
start = dom.find('<pre id="log"')
print(dom[start:start + 400])
