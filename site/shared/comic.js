// The demo as a comic strip: four panels per room, drawn from the same room scripts as the app demos.
import { ROOMS } from "./demo.js?v=ef5a40d8dd";
import { mount } from "./ember.js?v=ef5a40d8dd";
const still = matchMedia("(prefers-reduced-motion: reduce)");
const wait = (ms) => new Promise((r) => setTimeout(r, still.matches ? 0 : ms));
const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const pick = (room, kind) => room.script.filter((l) => l[0] === kind).map((l) => l[1]);
const STK = { lisbon: "c-magnify", morning: "c-mail", home: "c-notebook" };
const CAP = {
  lisbon: ["9:40 pm · you ask", "it gets to work", "it found one", "7:30 am · it waited for your OK"],
  morning: ["8:30 am · a routine", "it reads for you", "your brief", "the rest of the day"],
  home: ["you say", "it writes it down", "it answers", "what it knows now"],
};
function panels(id, answered, art = true) {
  const r = ROOMS[id], cap = CAP[id], work = pick(r, "work");
  const first = id === "morning" ? `<p class="sticky">${esc(pick(r, "meta")[0])}</p>` : `<p class="bub you">${esc(pick(r, "you")[0])}</p>`;
  const second = `${art ? `<img class="pstk" src="art/${STK[id]}-280.webp" width="140" height="140" alt="">` : ""}<ul class="ticklist">${work.map((w) => `<li>${esc(w)}</li>`).join("")}</ul>`;
  const third = `<div class="bub agent"><span class="mini" data-ember data-skin="mini" data-hop="off"></span><p>${esc(pick(r, "agent")[0])}</p></div>`;
  let fourth;
  if (id === "lisbon") {
    fourth = answered
      ? `<p class="result">${esc(r[answered][r[answered].length - 1][1])}</p><button type="button" class="undo" data-undo>Undo</button>`
      : `<div class="ask"><p class="ask-h">Lisbon trip needs you</p><p class="ask-q">${esc(r.ask.q)}</p><p class="ask-d">${esc(r.ask.d)}</p><div class="ask-b"><button type="button" class="btn-yes" data-answer="yes">Go ahead</button><button type="button" data-answer="no">Stop</button></div></div>`;
  } else if (id === "morning") {
    fourth = `${art ? `<img class="pstk" src="art/c-sleep-280.webp" width="140" height="140" alt="">` : ""}<p class="result">It keeps watching, and speaks up when something matters.</p>`;
  } else {
    fourth = `<ul class="memory-ui"><li class="fact is-new"><span class="fact-k">People</span><span class="fact-t">Mum’s birthday is 3 March</span><button type="button" class="fact-x" data-forget aria-label="Forget this: Mum's birthday is 3 March">Forget</button></li></ul>`;
  }
  return [first, second, third, fourth].map((body, i) => `<li class="panel p${i + 1}"><p class="pcap">${cap[i]}</p><div class="pbody">${body}</div></li>`);
}
export function comic(root, { onAsk, onDone, onUndo, start = "lisbon", art = true } = {}) {
  const list = root.querySelector("[data-panels]");
  let token = 0, current = start, answered = null;
  async function show(id) {
    const my = ++token; current = id;
    root.querySelectorAll("[data-room]").forEach((b) => b.setAttribute("aria-current", b.dataset.room === id ? "true" : "false"));
    list.innerHTML = "";
    for (const html of panels(id, id === "lisbon" ? answered : null, art)) {
      if (my !== token) return;
      list.insertAdjacentHTML("beforeend", html);
      list.lastElementChild.querySelectorAll("[data-ember]").forEach(mount);
      await wait(650);
    }
    if (id === "lisbon" && !answered) onAsk?.();
  }
  root.addEventListener("click", (e) => {
    const t = e.target.closest("[data-room]"); if (t && t.dataset.room !== current) { show(t.dataset.room); return; }
    const a = e.target.closest("[data-answer]");
    if (a) { answered = a.dataset.answer; onDone?.(answered); a.closest(".ask")?.remove(); const p4 = list.querySelector(".p4 .pbody"); const r = ROOMS.lisbon[answered];
      p4.innerHTML = `<p class="result">${esc(r[r.length - 1][1])}</p><button type="button" class="undo" data-undo>Undo</button>`; return; }
    if (e.target.closest("[data-undo]")) { answered = null; onUndo?.(); show("lisbon"); return; }
    const f = e.target.closest("[data-forget]"); if (f) { const li = f.closest(".fact"); li.classList.add("is-gone"); setTimeout(() => { li.parentElement.dataset.empty = ""; li.remove(); }, 300); }
  });
  const io = new IntersectionObserver((es) => { if (es.some((x) => x.isIntersecting)) { io.disconnect(); show(start); } }, { threshold: .25 });
  io.observe(root);
  return {
    answer: (a) => { answered = a; if (current === "lisbon" && list.querySelector(".p4 .pbody")) { const r = ROOMS.lisbon[a]; list.querySelector(".p4 .pbody").innerHTML = `<p class="result">${esc(r[r.length - 1][1])}</p><button type="button" class="undo" data-undo>Undo</button>`; } },
    undo: () => { answered = null; if (current === "lisbon") show("lisbon"); },
  };
}
