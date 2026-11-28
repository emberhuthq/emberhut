// The consent page: before a first pairing, what pairing sends and to whom (disclosure.ts), and the owner's Pair or
// Not now. It acts on the offer the service worker holds (pairing-flow.ts), never on an address of its own.

import { disclosure } from "../disclosure";
import { messageOf } from "../log";
import { refusal, type Answer, type PopupAsk, type Status } from "../status";

const $ = (id: string): HTMLElement => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`the page has no #${id}`);
  return element;
};
const show = (id: string, on: boolean): void => void $(id).classList.toggle("hide", !on);

async function ask(message: PopupAsk): Promise<Status> {
  const answer = await chrome.runtime.sendMessage<PopupAsk, Answer>(message);
  const refused = refusal(answer);
  if (refused !== null) throw new Error(refused);
  return answer as Status;
}

function render(s: Status): void {
  if (over) return;
  show("ask", s.offer !== null);
  show("none", s.offer === null && !s.paired);
  $("server").textContent = s.offer?.server ?? s.server ?? "";
  if (s.offer)
    $("lines").replaceChildren(
      ...disclosure(s.offer.server).map((line) => Object.assign(document.createElement("li"), { textContent: line })),
    );
}

let over = false;

function finished(text: string): void {
  over = true;
  show("ask", false);
  show("done", true);
  $("done").textContent = text;
  setTimeout(() => window.close(), 2_500);
}

$("pair").addEventListener("click", () => {
  $("err").textContent = "";
  ($("pair") as HTMLButtonElement).disabled = true;
  ask({ kind: "accept" }).then(
    (s) => finished(`Paired with ${s.server ?? "your hut"}. This tab closes by itself.`),
    (e: unknown) => {
      $("err").textContent = messageOf(e);
      ($("pair") as HTMLButtonElement).disabled = false;
    },
  );
});
$("later").addEventListener("click", () => {
  ask({ kind: "dismiss" }).then(
    () => finished("Not paired. Open the link from the Emberhut app again whenever you want to."),
    (e: unknown) => ($("err").textContent = messageOf(e)),
  );
});

// As the popup does: asked again every 2 s, since Chrome can drop a page's first message to a worker that is starting.
const refresh = (): void => {
  if (over) return;
  ask({ kind: "status" }).then(render, (e: unknown) => ($("err").textContent = messageOf(e)));
};
refresh();
setInterval(refresh, 2_000);
