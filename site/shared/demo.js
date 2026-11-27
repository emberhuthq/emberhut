// A small working model of the Emberhut app, true to how the real one behaves: rooms, one live line while the agent
// works, its reply, and the approval card above the message box ("Go ahead" / "Stop") that waits for you.
// Shared by every direction; each styles it its own way. Markup hooks: [data-feed], [data-status], [data-askslot],
// [data-room-title], [data-room-hint], [data-room] buttons, [data-needs-count], [data-live], [data-agent-ember].
import { mount, rest, hop } from "./ember.js?v=3f14f3d5e9";
const still = matchMedia("(prefers-reduced-motion: reduce)");
const wait = (ms) => new Promise((r) => setTimeout(r, still.matches ? 0 : ms));
const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

export const ROOMS = {
  lisbon: {
    name: "Lisbon trip", hint: "Flights for the 12th", dot: "#FF9500",
    script: [
      ["you", "Find me a morning flight to Lisbon on the 12th, under $250, and book it."],
      ["work", "Opening a flights site in your Chrome, Personal profile"],
      ["work", "Comparing 14 morning flights"],
      ["agent", "The best is the 9:40 nonstop for $212, with an aisle seat free. I've filled in everything up to paying."],
    ],
    ask: { q: "Book the 9:40 nonstop on 12 December for $212?", d: "The last step pays with the card saved in your Chrome." },
    yes: [["work", "Paying and checking the confirmation"], ["agent", "Booked. The confirmation is in your inbox."]],
    no: [["agent", "I left it unbooked. The flight is saved in this room if you change your mind."]],
  },
  morning: {
    name: "Morning brief", hint: "Routine · every morning at 8:30", dot: "#34C759",
    script: [
      ["meta", "Emberhut noticed · 41 new mails"],
      ["work", "Reading your mail and Slack"],
      ["agent", "Good morning. Three emails need a reply, the top one from your accountant. Sam asked in Slack whether Friday's report is on track."],
    ],
  },
  home: {
    name: "Home", hint: "Birthdays, bills, the boiler", dot: "#AF52DE",
    script: [
      ["you", "Remember that Mum's birthday is 3 March, and remind me a week before."],
      ["work", "Saving a fact about people you know"],
      ["agent", "Saved. I'll remind you on 24 February, with gift ideas if you want them."],
    ],
  },
};

export function demo(root, { onAsk, onDone, onWork, onUndo, onRoom, fast } = {}) {
  const feed = root.querySelector("[data-feed]");
  const status = root.querySelector("[data-status]");
  const slot = root.querySelector("[data-askslot]");
  const title = root.querySelector("[data-room-title]");
  const hint = root.querySelector("[data-room-hint]");
  const counts = root.querySelectorAll("[data-needs-count]");
  const live = root.querySelector("[data-live]");
  const av = root.querySelector("[data-agent-ember]");
  if (av) mount(av);
  const played = {}; let token = 0; let current = "lisbon"; let answered = null;
  const setNeeds = (n) => counts.forEach((b) => { b.textContent = n; b.hidden = !n; });
  setNeeds(1);

  const bubble = ([kind, text]) => {
    const li = document.createElement("li");
    li.className = `msg msg-${kind}`;
    if (kind === "meta") li.innerHTML = `<span>${esc(text)}</span>`;
    else if (kind === "agent") li.innerHTML = `<span class="who" aria-hidden="true"><span class="mini-ember" data-ember data-skin="mini" data-hop="off"></span></span><p><span class="sr">Emberhut: </span>${esc(text)}</p>`;
    else li.innerHTML = `<p><span class="sr">You: </span>${esc(text)}</p>`;
    li.querySelectorAll("[data-ember]").forEach(mount);
    return li;
  };
  const working = (text) => {
    if (!status) return;
    status.hidden = !text;
    status.innerHTML = text ? `<span class="spin" aria-hidden="true"></span><span>${esc(text)}</span>` : "";
    if (av) av.classList.toggle("is-busy", !!text);
    if (live) live.dataset.state = text ? "browsing" : (answered === "yes" ? "done" : "idle");
    onWork?.(!!text);
  };
  const askCard = () => {
    const r = ROOMS.lisbon.ask;
    slot.innerHTML = `<div class="ask" role="group" aria-label="Lisbon trip needs you"><p class="ask-h"><b>Lisbon trip needs you</b></p>` +
      `<p class="ask-q">${esc(r.q)}</p><p class="ask-d">${esc(r.d)}</p><div class="ask-b">` +
      `<button type="button" class="btn-yes" data-answer="yes">Go ahead</button><button type="button" class="btn-no" data-answer="no">Stop</button></div></div>`;
    slot.hidden = false;
    requestAnimationFrame(() => { feed.scrollTop = feed.scrollHeight; });
  };
  const toast = (text) => {
    slot.innerHTML = `<div class="toast" role="status"><span>${esc(text)}</span><button type="button" data-undo>Undo</button></div>`;
    slot.hidden = false;
  };

  async function play(id, fresh) {
    const my = ++token; current = id;
    const room = ROOMS[id];
    root.dataset.activeRoom = id;
    onRoom?.(id);
    if (title) title.textContent = room.name;
    if (hint) hint.textContent = room.hint;
    root.querySelectorAll("[data-room]").forEach((b) => b.setAttribute("aria-current", b.dataset.room === id ? "true" : "false"));
    feed.innerHTML = ""; if (slot) { slot.hidden = true; slot.innerHTML = ""; } working("");
    const lines = room.script.concat(id === "lisbon" && answered ? room[answered] : []);
    const quick = played[id] && !fresh;
    for (const l of lines) {
      if (my !== token) return;
      if (l[0] === "work") { if (!quick) { working(l[1]); await wait(fast && !played[id] ? 650 : 1300); } continue; }
      if (!quick) await wait(l[0] === "you" ? (fast ? 150 : 500) : 250);
      if (my !== token) return;
      working(""); feed.append(bubble(l)); feed.scrollTop = feed.scrollHeight;
    }
    working("");
    if (id === "home" && my === token) document.dispatchEvent(new CustomEvent("ember:remember", { detail: { id: "mum", kind: "People", text: "Mum's birthday is 3 March" } }));
    played[id] = true;
    if (id === "lisbon" && !answered && my === token) { askCard(); if (av) rest(av, "ooh"); onAsk?.(); }
  }
  async function answer(a, quiet) {
    answered = a; setNeeds(0);
    if (current !== "lisbon" || !played.lisbon) { onDone?.(a, quiet); return; }   // shows when the room plays
    if (av) { rest(av, "calm"); if (a === "yes") hop(av); }
    toast(`Answered: ${a === "yes" ? "Go ahead" : "Stop"}`);
    for (const l of ROOMS.lisbon[a]) {
      if (l[0] === "work") { working(l[1]); await wait(1100); continue; }
      working(""); feed.append(bubble(l)); feed.scrollTop = feed.scrollHeight;
    }
    working("");
    onDone?.(a, quiet);
  }
  function undo(quiet) {
    answered = null; setNeeds(1);
    if (current === "lisbon" && played.lisbon) play("lisbon", false);
    if (!quiet) onUndo?.();
  }
  root.addEventListener("click", (e) => {
    const r = e.target.closest("[data-room]"); if (r) { if (r.dataset.room !== current) play(r.dataset.room); return; }
    const a = e.target.closest("[data-answer]"); if (a && !answered) { answer(a.dataset.answer); return; }
    if (e.target.closest("[data-undo]")) undo();
  });
  const io = new IntersectionObserver((es) => { if (es.some((x) => x.isIntersecting)) { io.disconnect(); play("lisbon", true); } }, { threshold: .3 });
  io.observe(root);
  // for a second ask of the same booking elsewhere on the page: answer or undo it from outside, without echoing back
  return { play, answer: (a) => { if (!answered) answer(a, true); }, undo: () => { if (answered) undo(true); }, get answered() { return answered; } };
}
