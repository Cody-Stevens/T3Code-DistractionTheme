// Paste into T3's DevTools console (Ctrl+Shift+I, Console tab) while GlowTube is playing.
// It describes where the GlowTube clip image is and everything around it that could hide it,
// then copies the result to the clipboard so it can be pasted back to the agent.
(() => {
  const props = ["display", "visibility", "opacity", "position", "zIndex", "overflow", "transform", "translate",
    "scale", "rotate", "perspective", "filter", "backdropFilter", "contain", "containerType", "contentVisibility",
    "maskImage", "webkitMaskImage", "clipPath", "willChange", "isolation", "mixBlendMode"];
  const quiet = { display: "block", visibility: "visible", opacity: "1", position: "static", zIndex: "auto",
    overflow: "visible", transform: "none", translate: "none", scale: "none", rotate: "none", perspective: "none",
    filter: "none", backdropFilter: "none", contain: "none", containerType: "normal", contentVisibility: "visible",
    maskImage: "none", webkitMaskImage: "none", clipPath: "none", willChange: "auto", isolation: "auto", mixBlendMode: "normal" };
  const describe = (el) => {
    const cs = getComputedStyle(el);
    const loud = {};
    for (const p of props) if (cs[p] !== undefined && cs[p] !== quiet[p]) loud[p] = String(cs[p]).slice(0, 60);
    const r = el.getBoundingClientRect();
    return { el: el.tagName.toLowerCase() + (el.dataset.slot ? `[slot=${el.dataset.slot}]` : "") +
      ([...el.attributes].filter(a => a.name.startsWith("data-") && a.name !== "data-slot").map(a => `[${a.name}]`).join("")),
      cls: String(el.className).slice(0, 90), rect: [r.x, r.y, r.width, r.height].map(Math.round), styles: loud };
  };
  const imgs = [...document.querySelectorAll('img[src*="glowtube-clip-"]')];
  const out = { ua: navigator.userAgent, viewport: [innerWidth, innerHeight], dpr: devicePixelRatio,
    htmlClass: document.documentElement.className, count: imgs.length, images: [] };
  for (const im of imgs) {
    const r = im.getBoundingClientRect();
    const cx = r.x + r.width / 2, cy = r.y + r.height / 2;
    const top = document.elementFromPoint(cx, cy);
    const chain = [];
    for (let el = im; el && el !== document.documentElement; el = el.parentElement) chain.push(describe(el));
    out.images.push({ src: im.src.slice(-40), complete: im.complete, natural: [im.naturalWidth, im.naturalHeight],
      topmostAtCentre: top ? describe(top) : null, chain });
  }
  const text = JSON.stringify(out, null, 1);
  copy(text);
  console.log("GlowTube check copied to the clipboard (" + text.length + " characters).");
  return out;
})();
