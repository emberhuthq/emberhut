import "./shared/reveal.js?v=ef5a40d8dd";
import "./shared/copy.js?v=ef5a40d8dd";
import "./shared/download.js?v=ef5a40d8dd";
import { mountAll, hop, rest } from "./shared/ember.js?v=ef5a40d8dd";
import { demo } from "./shared/demo.js?v=ef5a40d8dd";
import { comic } from "./shared/comic.js?v=ef5a40d8dd";
mountAll();

// the ember perched on the app window reacts to what happens inside it (no speech: the page says it)
const perch = document.querySelector(".perch-ember");
perch?.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); hop(perch); } });

// one booking, one answer: the app in the hero and the Lisbon strip below share it
const needs = document.querySelector("[data-comic-needs]");
const app = document.querySelector("[data-demo]");
const me = app?.querySelector("[data-me-status]");
let strip = null;
const hero = demo(app, {
  fast: true,
  onWork: (on) => { if (me) me.textContent = on ? "Working…" : "Watching"; app.toggleAttribute("data-working", on); },
  onAsk: () => rest(perch, "ooh"),
  onDone: (a, quiet) => {
    if (needs) needs.hidden = true;
    if (!quiet) strip?.answer(a);
    rest(perch, "calm");
    if (a === "yes") hop(perch);
  },
  onUndo: () => { if (needs) needs.hidden = false; strip?.undo(); rest(perch, "ooh"); },
});
strip = comic(document.querySelector("[data-comic]"), {
  art: false,
  start: "morning",
  onDone: (a) => { if (needs) needs.hidden = true; hero.answer(a); },
  onUndo: () => { if (needs) needs.hidden = false; hero.undo(); },
});

// the ember in the last call says hello
const fe = document.querySelector(".final-ember");
fe?.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); hop(fe); } });
