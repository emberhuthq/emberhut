// Passwords: Emberhut never types into or fills in a password or a one-time-code field. What the guard covers is the input
// and fill commands: before a command that types (inserted text, a key that carries text), the focused element is
// checked; before a function run on an element (how the server fills a field, reads it or acts on it), that element is
// checked. Both checks run in the pill's isolated world, which the page's scripts can't reach, and count a field that
// was a password field in this document (its type changed since) as one. A secret field, or a focus that can't be
// checked (another site's frame), refuses the command with "hand it to the owner". Script the server evaluates in the
// page is not read here: the server's own rules cover it (it blanks password values in everything it reads, and its
// step gate holds final steps). Paste, editing commands on a key, drag and drop and direct DOM edits are refused
// outright (cdp-rules.ts).

import { SECRET_REFUSED, SECRET_UNKNOWN, onElement, typesText } from "../cdp-rules";
import { ELEMENT_CHECK, FOCUS_CHECK, PILL_WORLD } from "../page-scripts";
import { mainFrame, noteOwnContext, pillWorld } from "./held";

interface Evaluated {
  result?: { value?: unknown; objectId?: string };
  exceptionDetails?: unknown;
}
interface FrameTree {
  frameTree: { frame: { id: string } };
}
interface World {
  executionContextId: number;
}
interface Described {
  node: { backendNodeId: number };
}
interface Resolved {
  object: { objectId?: string };
}

/** A new isolated world in the tab's top frame (the pill's script runs in it on every new document): its context. */
async function newWorld(tabId: number): Promise<number> {
  const tree = await chrome.debugger.sendCommand<object, FrameTree>({ tabId }, "Page.getFrameTree");
  const frameId = tree?.frameTree.frame.id;
  if (!frameId) throw new Error("no frame");
  const world = await chrome.debugger.sendCommand<object, World>({ tabId }, "Page.createIsolatedWorld", { frameId, worldName: PILL_WORLD });
  if (!world) throw new Error("no world");
  mainFrame.set(tabId, frameId);
  pillWorld.set(tabId, world.executionContextId);
  noteOwnContext(tabId, world.executionContextId);
  return world.executionContextId;
}

/** Runs `check` with the tab's pill world (made again once when the one known is gone, after a navigation). */
async function inWorld<T>(tabId: number, check: (contextId: number) => Promise<T | undefined>): Promise<T | undefined> {
  for (const known of [pillWorld.get(tabId), undefined]) {
    try {
      const answer = await check(known ?? (await newWorld(tabId)));
      if (answer !== undefined) return answer;
    } catch {
      // A world from before a navigation: made again on the next round.
    }
  }
  return undefined;
}

/** What has the focus in the tab: "secret", "plain" or "unknown". */
async function focus(tabId: number): Promise<string> {
  const answer = await inWorld(tabId, async (contextId) => {
    const got = await chrome.debugger.sendCommand<object, Evaluated>({ tabId }, "Runtime.evaluate", {
      expression: FOCUS_CHECK,
      contextId,
      returnByValue: true,
    });
    const value = got?.result?.value;
    return !got?.exceptionDetails && typeof value === "string" ? value : undefined;
  });
  return answer ?? "unknown";
}

/** Whether the element `objectId` names is, or was, a password or code field: checked in the pill's world. */
async function secretElement(tabId: number, objectId: string): Promise<boolean> {
  const tab = { tabId };
  const described = await chrome.debugger.sendCommand<object, Described>(tab, "DOM.describeNode", { objectId });
  const backendNodeId = described?.node.backendNodeId;
  if (backendNodeId === undefined) return true;
  const answer = await inWorld(tabId, async (executionContextId) => {
    const resolved = await chrome.debugger.sendCommand<object, Resolved>(tab, "DOM.resolveNode", { backendNodeId, executionContextId });
    const there = resolved?.object.objectId;
    if (!there) return undefined;
    try {
      const got = await chrome.debugger.sendCommand<object, Evaluated>(tab, "Runtime.callFunctionOn", {
        objectId: there,
        functionDeclaration: ELEMENT_CHECK,
        returnByValue: true,
      });
      const value = got?.result?.value;
      return typeof value === "boolean" ? value : undefined;
    } finally {
      await chrome.debugger.sendCommand(tab, "Runtime.releaseObject", { objectId: there }).catch(() => undefined);
    }
  });
  return answer ?? true;
}

/** Throws the owner's words when `method` would type into, or act on, a password or code field. */
export async function refuseSecretInput(tabId: number, method: string, params: unknown): Promise<void> {
  if (typesText(method, params)) {
    const where = await focus(tabId);
    if (where === "secret") throw new Error(SECRET_REFUSED);
    if (where !== "plain") throw new Error(SECRET_UNKNOWN);
  } else if (onElement(method, params)) {
    const objectId = (params as { objectId: string }).objectId;
    if (await secretElement(tabId, objectId).catch(() => true)) throw new Error(SECRET_REFUSED);
  }
}
