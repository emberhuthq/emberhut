// The popup: pair, pause Emberhut (it comes back by itself), share a tab, hand back, and see what Emberhut did. Everything
// goes through the service worker.

import { messageOf } from "../log";
import { blockedList } from "../scope";
import type { PopupAsk, Status } from "../status";
import { clock } from "../pause";
import { builtIn, driving, headline, pauseEnds } from "./view";

const $ = <T extends HTMLElement = HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`the popup has no #${id}`);
  return element as T;
};
const show = (id: string, on: boolean): void => void $(id).classList.toggle("hide", !on);

/** The worker's answer, or why none came: Chrome can lose an extension's worker. Pairing waits up to 15 s. */
function ask(message: PopupAsk): Promise<Status | { error: string }> {
  const wait = message.kind === "pair" || message.kind === "accept" ? 15_000 : 5_000;
  return Promise.race([
    chrome.runtime.sendMessage<PopupAsk, Status | { error: string }>(message),
    new Promise<never>((_, no) => setTimeout(() => no(new Error("no answer")), wait)),
  ]);
}

/** The worker isn't answering: say so, and reload the extension once a minute at most (its pairing stays). */
function dead(cause: unknown): void {
  $("status").textContent = `Emberhut isn't running in this Chrome (${messageOf(cause)})`;
  const last = Number(localStorage.getItem("reloaded") || 0);
  if (Date.now() - last < 60_000) {
    $("err").textContent = "Reload Emberhut at chrome://extensions in this profile.";
    return;
  }
  localStorage.setItem("reloaded", String(Date.now()));
  $("status").textContent = "Emberhut isn't running in this Chrome: starting it again…";
  setTimeout(() => chrome.runtime.reload(), 300);
}

async function send(message: PopupAsk): Promise<void> {
  $("err").textContent = "";
  try {
    const answer = await ask(message);
    if ("error" in answer) {
      $("err").textContent = answer.error;
      await send({ kind: "status" });
      return;
    }
    render(answer);
  } catch (cause) {
    dead(cause);
  }
}

function render(s: Status): void {
  show("offer", s.offer !== null);
  $("offerServer").textContent = s.offer?.server ?? "";
  show("pair", !s.paired && s.offer === null);
  show("main", s.paired);
  $("status").textContent = headline(s);
  if (s.error) $("err").textContent = s.error;
  if (!s.paired) return;
  show("off", s.paused !== null);
  show("in", s.paused === null);
  $("offWhy").textContent = s.paused?.why ?? "";
  $("offUntil").textContent = pauseEnds(s);
  $("inText").textContent = driving(s);
  show("need", s.handoff !== null);
  $("needWhy").textContent = s.handoff ?? "";
  show("notice", s.notice !== null && s.handoff === null);
  $("notice").textContent = s.notice ?? "";
  $("tabs").replaceChildren(...s.tabs.map((t) => Object.assign(document.createElement("li"), { textContent: t.title || t.url })));
  show("tabsBox", s.tabs.length > 0);
  const steps = s.steps.length ? s.steps : [{ at: 0, text: "Nothing yet" }];
  $("steps").replaceChildren(
    ...steps.map((x) => {
      const li = document.createElement("li");
      if (x.at) li.append(Object.assign(document.createElement("time"), { textContent: clock(x.at) }));
      li.append(x.text);
      return li;
    }),
  );
  const box = $<HTMLTextAreaElement>("blocked");
  if (document.activeElement !== box) box.value = s.blocked.join("\n");
  $("builtin").textContent = builtIn(s);
}

const on = (id: string, message: () => PopupAsk | null): void => {
  $(id).addEventListener("click", () => {
    const m = message();
    if (m) send(m).catch(dead);
  });
};

on("doPair", () => ({ kind: "pair", code: $<HTMLInputElement>("code").value, server: $<HTMLInputElement>("server").value }));
on("accept", () => ({ kind: "accept" }));
on("dismiss", () => ({ kind: "dismiss" }));
on("pause", () => ({ kind: "pause" }));
on("share", () => ({ kind: "share" }));
on("handback", () => ({ kind: "handback" }));
on("save", () => ({ kind: "settings", blocked: blockedList($<HTMLTextAreaElement>("blocked").value) }));
on("unpair", () => (confirm("Unpair this Chrome profile from Emberhut?") ? { kind: "unpair" } : null));

send({ kind: "status" }).catch(dead);
setInterval(() => {
  ask({ kind: "status" }).then((answer) => ("error" in answer ? undefined : render(answer)), dead);
}, 2_000);
