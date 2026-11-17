"""Bisect which favicon-overlay CSS rules the desktop app needs.

  python tools/try_rules.py <variant>

Rewrites the overlay rules in theme_css.py to the named variant (see VARIANTS); the helper must be
restarted to pick it up."""
import re
import sys
from pathlib import Path

P = Path(__file__).resolve().parents[1] / "theme_css.py"
SEL = 'img[src*="glowtube-clip-"]'
RULES = {
    "mask": '[data-app-sidebar] [data-slot="scroll-area-viewport"]:has(' + SEL + '){{-webkit-mask:none!important;mask:none!important}}',
    "anymask": '*:has(' + SEL + '){{-webkit-mask:none!important;mask:none!important;clip-path:none!important}}',
    "transform": '*:has(' + SEL + '){{transform:none!important;translate:none!important;scale:none!important;rotate:none!important;perspective:none!important}}',
    "filter": '*:has(' + SEL + '){{backdrop-filter:none!important;filter:none!important;will-change:auto!important;contain:none!important;container-type:normal!important;content-visibility:visible!important}}',
    "zindex": '[data-slot="sidebar-container"]:has(' + SEL + '){{z-index:2147483000!important}}',
}
VARIANTS = {
    "mask": ["mask"], "anymask": ["anymask"], "zindex": ["zindex"], "mask+zindex": ["mask", "zindex"],
    "anymask+zindex": ["anymask", "zindex"], "transform": ["transform"], "filter": ["filter"],
    "all": ["anymask", "transform", "filter", "zindex"],
}

variant = sys.argv[1]
s = P.read_text(encoding="utf-8")
begin, end = "/* overlay-rules-begin */\n", "/* overlay-rules-end */\n"
block = "".join(RULES[name] + "\n" for name in VARIANTS[variant])
if begin in s:
    s = re.sub(re.escape(begin) + ".*?" + re.escape(end), lambda m: begin + block + end, s, flags=re.S)
else:
    anchor = 'img[src*="glowtube-clip-"]{{outline'
    s = s.replace(anchor, begin + block + end + anchor, 1)
P.write_text(s, encoding="utf-8")
print("rules now:", variant)
