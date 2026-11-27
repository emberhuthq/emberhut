// The living ember: builds the layers into every [data-ember] element, blinks and breathes in CSS, and adds the
// pointer-following eyes, the hop on tap and a talk() helper. Nothing moves under prefers-reduced-motion.
const BASE = new URL("ember/", import.meta.url).href;
const still = matchMedia("(prefers-reduced-motion: reduce)");
const L = (cls, f) => `<img class="${cls}" src="${BASE}${f}.svg" alt="" width="200" height="200" decoding="async">`;
export function mount(el) {
  if (el.dataset.ready) return el; el.dataset.ready = "1";
  el.classList.add("ember");
  const skin = el.dataset.skin;   // "flat" (poster) or "soft"; the face is the same character everywhere
  const body = skin === "flat" ? "body-flat" : skin === "soft" ? "body-soft" : skin === "mini" ? "body-mini" : "body";
  const eyes = skin === "flat" || skin === "mini" ? "eyes-bold" : "eyes";
  el.insertAdjacentHTML("afterbegin",
    `<div class="ember-glow"></div><div class="ember-hop"><div class="ember-bob">${el.hasAttribute("data-sticker") ? L("e-back", "sticker-back") : ""}${L("e-body", body)}` +
    `<div class="ember-look"><div class="ember-lids">${L("e-open", eyes)}${L("e-happy", "eyes-happy")}</div>${L("e-closed", "eyes-closed")}</div>` +
    `<div class="ember-mouths">${L("m-smile", "mouth")}${L("m-open", "mouth-open")}${L("m-happy", "mouth-happy")}${L("m-ooh", "mouth-ooh")}</div></div></div>`);
  if (el.dataset.hop !== "off") {
    el.addEventListener("click", () => hop(el));
  }
  return el;
}
export function hop(el) {
  if (still.matches) { mood(el, "happy", 1200); return; }
  el.classList.remove("is-hopping"); void el.offsetWidth; el.classList.add("is-hopping");
  mood(el, "happy", 900);
  setTimeout(() => el.classList.remove("is-hopping"), 700);
}
export function mood(el, m, ms) {
  const prev = el.dataset.rest || el.dataset.mood || "calm";
  if (!el.dataset.rest) el.dataset.rest = prev;
  el.dataset.mood = m;
  clearTimeout(el._moodT);
  if (ms) el._moodT = setTimeout(() => { el.dataset.mood = el.dataset.rest; }, ms);
}
export function rest(el, m) { el.dataset.rest = m; el.dataset.mood = m; }
// Type text into a bubble while the ember talks. Resolves when done.
export function talk(el, bubble, text, cps = 38) {
  return new Promise((done) => {
    clearInterval(bubble._t);
    if (still.matches || !text) { bubble.textContent = text; done(); return; }
    el.classList.add("is-talking"); bubble.textContent = "";
    let i = 0;
    bubble._t = setInterval(() => {
      i += 1 + (text[i] === " " ? 1 : 0);
      bubble.textContent = text.slice(0, i);
      if (i >= text.length) { clearInterval(bubble._t); el.classList.remove("is-talking"); done(); }
    }, 1000 / cps);
  });
}
// Eyes follow the pointer (any ember within ~900px), throttled to one update a frame.
let raf = 0, px = 0, py = 0;
function look() {
  raf = 0;
  for (const el of document.querySelectorAll(".ember[data-look]")) {
    const r = el.getBoundingClientRect();
    if (r.bottom < 0 || r.top > innerHeight) continue;
    const dx = px - (r.left + r.width / 2), dy = py - (r.top + r.height * .52);
    const d = Math.hypot(dx, dy) || 1, k = Math.min(1, d / 420);
    el.style.setProperty("--lx", ((dx / d) * k).toFixed(3));
    el.style.setProperty("--ly", ((dy / d) * k).toFixed(3));
  }
}
if (!still.matches) addEventListener("pointermove", (e) => { px = e.clientX; py = e.clientY; if (!raf) raf = requestAnimationFrame(look); }, { passive: true });
export function mountAll(root = document) { root.querySelectorAll("[data-ember]").forEach(mount); }
