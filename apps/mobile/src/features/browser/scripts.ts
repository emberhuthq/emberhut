// The live view's page (the stream's own web client) gets these ahead of its own scripts, as the reference client's does.

/** Only the page: the stream's floating keyboard button and the handle of its settings drawer stay hidden. */
const streamOnly = `
(() => { const s = document.createElement("style"); s.textContent = ".virtual-keyboard-button, .keyboard-toggle-button, .toggle-handle { display: none !important; }";
  (document.head || document.documentElement).appendChild(s); })();
`;

/**
 * Watching while nobody drives: the stream tells a viewer the screen's size only while someone drives, and without it the
 * picture is squeezed to the phone's shape. The app knows the size, so the page is told it unless the stream does first.
 * The size comes from the server's answer and goes into code: anything but two positive numbers gives no script at all.
 */
export function screenSize(width: number, height: number): string {
  const w = Math.round(Number(width));
  const h = Math.round(Number(height));
  if (!Number.isFinite(w) || !Number.isFinite(h) || w <= 0 || h <= 0) return "";
  return `
(() => {
  // The stream's connection (a worker's socket, or a plain one) is published as window.selkiesTransport; its page
  // handler is set as onmessage. Ours sits in front of it: it notes whether the stream sent a size, and if it didn't,
  // hands the page the size itself.
  let transport = null;
  Object.defineProperty(window, "selkiesTransport", { configurable: true, get: () => transport, set: (t) => {
    transport = t;
    if (!t) return;
    let handler = null, told = false;
    Object.defineProperty(t, "onmessage", { configurable: true, get: () => handler, set: (fn) => {
      handler = fn && ((e) => { if (typeof e.data === "string" && e.data.includes('"stream_resolution"')) told = true; return fn.call(t, e); });
      // The page is listening: a size unless the stream gives one; again a little later in case it wasn't ready yet
      // (the same size twice changes nothing). Pictures may not pass here (a worker can decode them), so on time.
      if (fn) for (const ms of [1000, 2500, 5000, 9000]) setTimeout(() => {
        if (!told && handler) fn.call(t, { data: JSON.stringify({ type: "stream_resolution", width: ${w}, height: ${h}, displayId: "primary" }) });
      }, ms);
    } });
  } });
})();
`;
}

/** While you drive, from the app's own controls: a key with modifiers, text typed on the phone's keyboard, a paste. */
const driveHelpers = `
(() => {
  const send = (t, type, init) => t.dispatchEvent(new KeyboardEvent(type, Object.assign({ bubbles: true, cancelable: true, composed: true }, init)));
  const where = () => document.activeElement || document.body;
  const mods = [["ctrl", { key: "Control", code: "ControlLeft", keyCode: 17 }], ["shift", { key: "Shift", code: "ShiftLeft", keyCode: 16 }], ["alt", { key: "Alt", code: "AltLeft", keyCode: 18 }]];
  window.__emberhutKey = (key, code, keyCode, m) => {
    const t = where(), on = mods.filter(([n]) => m && m[n]), flags = { ctrlKey: !!(m && m.ctrl), shiftKey: !!(m && m.shift), altKey: !!(m && m.alt) };
    for (const [, k] of on) send(t, "keydown", { ...k, ...flags });
    send(t, "keydown", { key, code, keyCode, which: keyCode, ...flags }); send(t, "keyup", { key, code, keyCode, which: keyCode, ...flags });
    for (const [, k] of on.reverse()) send(t, "keyup", { ...k });
  };
  window.__emberhutText = (s) => { for (const ch of s) { const c = ch.toUpperCase().charCodeAt(0); window.__emberhutKey(ch, "", c, { shift: ch !== ch.toLowerCase() }); } };
  // The stream takes the clipboard from a paste event, then Ctrl+V pastes it on the far machine (it only presses that itself
  // for a real keyboard's paste).
  window.__emberhutPaste = (text) => {
    const d = new DataTransfer(); d.setData("text/plain", text);
    window.dispatchEvent(new ClipboardEvent("paste", { clipboardData: d, bubbles: true, cancelable: true }));
    setTimeout(() => window.__emberhutKey("v", "KeyV", 86, { ctrl: true }), 300);
  };
})();
`;

/**
 * Your fingers as a mouse, ahead of the stream's own touch handling: tap = click (twice = double-click), long press =
 * right-click, long press then drag = select, drag = scroll with momentum, two fingers zoom and pan the view itself.
 */
const touchAsMouse = `
(() => {
  let zoom = 1, tx = 0, ty = 0, one = null, two = null, spin = null;
  const apply = () => { const b = document.body; b.style.transformOrigin = "0 0"; b.style.transform = zoom === 1 ? "" : \`translate(\${tx}px, \${ty}px) scale(\${zoom})\`; };
  const at = (x, y) => document.elementFromPoint(x, y) || document.body;
  const mouse = (type, x, y, button, buttons, detail) => (type === "mousedown" ? at(x, y) : window).dispatchEvent(new MouseEvent(type,
    { clientX: x, clientY: y, button, buttons, detail, bubbles: true, cancelable: true, view: window }));
  const click = (x, y, button, detail) => { mouse("mousemove", x, y, 0, 0, 0); mouse("mousedown", x, y, button, button === 2 ? 2 : 1, detail); mouse("mouseup", x, y, button, 0, detail); };
  const wheel = (x, y, dx, dy) => at(x, y).dispatchEvent(new WheelEvent("wheel", { clientX: x, clientY: y, deltaX: dx, deltaY: dy, deltaMode: 0, bubbles: true, cancelable: true }));
  let lastTap = 0;
  // The stream's own controls keep their touches; its typing overlay (an input laid over the picture) is the picture.
  const own = (e) => !e.target.closest || e.target.id === "overlayInput" || !e.target.closest("button, input, select, textarea, a, [role=button], .sidebar, .menu");
  const stop = (e) => { e.preventDefault(); e.stopImmediatePropagation(); };
  window.addEventListener("touchstart", (e) => {
    if (!own(e)) return; stop(e);
    if (spin) { cancelAnimationFrame(spin); spin = null; }
    if (e.touches.length === 1) {
      const t = e.touches[0];
      one = { x: t.clientX, y: t.clientY, lx: t.clientX, ly: t.clientY, t: Date.now(), moved: false, mode: "tap", vy: 0, vx: 0, lt: Date.now() };
      one.timer = setTimeout(() => { if (one && !one.moved) one.mode = "long"; }, 500);
    } else if (e.touches.length === 2) {
      if (one) clearTimeout(one.timer); one = null;
      const [a, b] = e.touches, mx = (a.clientX + b.clientX) / 2, my = (a.clientY + b.clientY) / 2;
      two = { d: Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY), z: zoom, mx, my, tx, ty };
    }
  }, { capture: true, passive: false });
  window.addEventListener("touchmove", (e) => {
    if (!own(e)) return; stop(e);
    if (two && e.touches.length === 2) {
      const [a, b] = e.touches, mx = (a.clientX + b.clientX) / 2, my = (a.clientY + b.clientY) / 2;
      zoom = Math.min(4, Math.max(1, two.z * Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY) / two.d));
      // the point under your fingers stays under them, and moves with them
      tx = mx - (two.mx - two.tx) * zoom / two.z; ty = my - (two.my - two.ty) * zoom / two.z;
      if (zoom === 1) { tx = 0; ty = 0; }
      apply(); return;
    }
    if (!one) return;
    const t = e.touches[0], dx = t.clientX - one.lx, dy = t.clientY - one.ly, now = Date.now();
    if (!one.moved && Math.hypot(t.clientX - one.x, t.clientY - one.y) > 10) {
      one.moved = true; clearTimeout(one.timer);
      if (one.mode === "long") { one.mode = "select"; mouse("mousedown", one.x, one.y, 0, 1, 1); } else one.mode = "scroll";
    }
    if (one.mode === "select") mouse("mousemove", t.clientX, t.clientY, 0, 1, 0);
    if (one.mode === "scroll") { wheel(t.clientX, t.clientY, -dx / zoom, -dy / zoom); const dt = Math.max(1, now - one.lt); one.vx = -dx / dt; one.vy = -dy / dt; }
    one.lx = t.clientX; one.ly = t.clientY; one.lt = now;
  }, { capture: true, passive: false });
  const end = (e) => {
    if (!own(e)) return; stop(e);
    if (two && e.touches.length < 2) { two = null; return; }
    if (!one) return;
    clearTimeout(one.timer);
    const g = one; one = null;
    if (g.mode === "long") click(g.x, g.y, 2, 1);
    else if (g.mode === "select") mouse("mouseup", g.lx, g.ly, 0, 0, 1);
    else if (g.mode === "tap") { const now = Date.now(), n = now - lastTap < 350 ? 2 : 1; lastTap = now; click(g.x, g.y, 0, n); }
    else if (g.mode === "scroll" && (Math.abs(g.vy) > 0.3 || Math.abs(g.vx) > 0.3)) { // momentum
      let vx = g.vx * 16, vy = g.vy * 16;
      const step = () => { vx *= 0.94; vy *= 0.94; if (Math.abs(vx) + Math.abs(vy) < 1) { spin = null; return; } wheel(g.lx, g.ly, vx / zoom, vy / zoom); spin = requestAnimationFrame(step); };
      spin = requestAnimationFrame(step);
    }
  };
  window.addEventListener("touchend", end, { capture: true, passive: false });
  window.addEventListener("touchcancel", end, { capture: true, passive: false });
  window.__emberhutResetZoom = () => { zoom = 1; tx = 0; ty = 0; apply(); };
  // Turned sideways (or back): the far machine's screen takes the new shape. The stream asks for its size only once, on
  // connecting (and the keyboard coming up keeps the size).
  let wait = null;
  screen.orientation.addEventListener("change", () => {
    clearTimeout(wait);
    wait = setTimeout(() => {
      const t = window.selkiesTransport, r = devicePixelRatio, w = Math.round(innerWidth * r / 2) * 2, h = Math.round(innerHeight * r / 2) * 2;
      if (t && t.readyState === 1) { t.send(\`r,\${w}x\${h},primary\`); }
    }, 400);
  });
})();
`;

/** What a key press runs in the page while you drive. */
export function keyScript(key: string, code: string, keyCode: number, mods: { ctrl?: boolean; shift?: boolean } = {}): string {
  return `window.__emberhutKey && __emberhutKey(${JSON.stringify(key)}, ${JSON.stringify(code)}, ${keyCode}, ${JSON.stringify({ ctrl: !!mods.ctrl, shift: !!mods.shift })}); true;`;
}

export const textScript = (text: string): string => `window.__emberhutText && __emberhutText(${JSON.stringify(text)}); true;`;
export const pasteScript = (text: string): string => `window.__emberhutPaste && __emberhutPaste(${JSON.stringify(text)}); true;`;
export const resetZoomScript = "window.__emberhutResetZoom && __emberhutResetZoom(); true;";

/** What runs before the page: view-only gets the size, control gets the keys and the fingers-as-mouse. */
export function injected(mode: "control" | "view", screen: { width: number; height: number } | null | undefined): string {
  const parts = [
    streamOnly,
    mode === "view" && screen ? screenSize(screen.width, screen.height) : "",
    mode === "control" ? driveHelpers + touchAsMouse : "",
  ];
  return `${parts.join("\n")}\ntrue;`;
}

/** The typing row's change as keys: what was added typed, what was removed deleted, a rewrite both. */
export function typedChange(before: string, after: string): { backspaces: number; text: string } {
  if (after.startsWith(before)) return { backspaces: 0, text: after.slice(before.length) };
  if (before.startsWith(after)) return { backspaces: before.length - after.length, text: "" };
  return { backspaces: before.length, text: after };
}
