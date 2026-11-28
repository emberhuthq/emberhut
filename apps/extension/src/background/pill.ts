// What Emberhut sets up in a tab it attaches to: the tab kept awake, the page's console quiet, the page-closed-itself
// binding, and the pill with its cursor (page-scripts.ts).

import { CLOSING_BINDING, GUARD, PILL, PILL_BINDING, PILL_WORLD, pointAt } from "../page-scripts";
import { logError, messageOf } from "../log";
import { mainFrame, noteOwnContext, pillWorld } from "./held";
import { within } from "./tabs";

interface FrameTree {
  frameTree: { frame: { id: string } };
}
interface World {
  executionContextId: number;
}

/**
 * Emberhut's tab acts as seen and focused while Emberhut holds it, wherever it is: behind the owner's tab, in a covered or
 * minimized window. Chrome draws no frames for a page nobody sees, and a click that waits for the element to hold
 * still over two frames would time out. DevTools' focus emulation counts as a capture of the tab, so the page runs as
 * if visible and focused while the owner's window, tab and focus don't move. Active: a tab Chrome froze in the
 * background runs again. Quiet: the page's console goes nowhere, from this document on, and what it logged before
 * Emberhut came is dropped. All of it ends with Emberhut's link to the tab, and lasts across the tab's navigations
 * until then.
 */
export async function awake(tabId: number): Promise<void> {
  const steps: [string, object][] = [
    // First, so the bindings and new-document scripts below outlive the page's navigations (Chrome drops them on the
    // next document from a session without these domains on) and the pill's Pause reaches the worker
    // (Runtime.bindingCalled), with the pill's world known (Runtime.executionContextCreated, main.ts).
    ["Page.enable", {}],
    ["Runtime.enable", {}],
    ["Emulation.setFocusEmulationEnabled", { enabled: true }],
    ["Page.setWebLifecycleState", { state: "active" }],
    ["Runtime.addBinding", { name: CLOSING_BINDING }],
    ["Page.addScriptToEvaluateOnNewDocument", { source: GUARD }],
    ["Runtime.evaluate", { expression: GUARD }],
    ["Runtime.discardConsoleEntries", {}],
  ];
  for (const [method, params] of steps) {
    await within(5_000, chrome.debugger.sendCommand({ tabId }, method, params)).catch((e: unknown) => logError(method, messageOf(e)));
  }
}

/** The pill (and the cursor's script) in the tab's isolated world, now and on every document after. */
export async function pill(tabId: number): Promise<void> {
  const tab = { tabId };
  await chrome.debugger.sendCommand(tab, "Runtime.addBinding", { name: PILL_BINDING, executionContextName: PILL_WORLD });
  await chrome.debugger.sendCommand(tab, "Page.addScriptToEvaluateOnNewDocument", { source: PILL, worldName: PILL_WORLD });
  const tree = await chrome.debugger.sendCommand<object, FrameTree>(tab, "Page.getFrameTree");
  const frameId = tree?.frameTree.frame.id;
  if (!frameId) return;
  const world = await chrome.debugger.sendCommand<object, World>(tab, "Page.createIsolatedWorld", { frameId, worldName: PILL_WORLD });
  if (!world) return;
  mainFrame.set(tabId, frameId);
  pillWorld.set(tabId, world.executionContextId);
  noteOwnContext(tabId, world.executionContextId);
  await chrome.debugger.sendCommand(tab, "Runtime.evaluate", { expression: PILL, contextId: world.executionContextId });
}

/** Emberhut's cursor to where its mouse event went (a page between worlds just doesn't show it this once). */
export function pointTo(tabId: number, event: unknown): void {
  const contextId = pillWorld.get(tabId);
  const e = (event ?? {}) as { x?: unknown; y?: unknown; type?: unknown };
  if (contextId === undefined || typeof e.x !== "number" || typeof e.y !== "number" || !Number.isFinite(e.x) || !Number.isFinite(e.y))
    return;
  chrome.debugger
    .sendCommand({ tabId }, "Runtime.evaluate", { expression: pointAt(e.x, e.y, String(e.type)), contextId })
    .catch(() => undefined);
}
