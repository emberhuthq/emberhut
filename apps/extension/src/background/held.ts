// What the service worker knows of Emberhut's tabs while it runs (a restarted worker starts empty: the server's next
// emberhut.start reports them again).

/** Tabs with the debugger on. */
export const attached = new Set<number>();
/** Tabs told to the server as Emberhut's. */
export const reported = new Set<number>();
/** Sign-in popups one of Emberhut's tabs opened: Emberhut's too. */
export const adopted = new Set<number>();
/** Tabs Emberhut opened, Emberhut's even before Chrome shows them in the group. */
export const made = new Set<number>();
/** Tabs joining the group right now (a new group is untitled for a moment): sync leaves them alone. */
export const joining = new Set<number>();
/** Who is about to close a tab (the page itself, Emberhut, a crash), said just before it went. */
export const closing = new Map<number, { by: string; at: number }>();
/** Where each of Emberhut's tabs is, for the server when it closes. */
export const urls = new Map<number, string>();
/** Tabs another extension's frame keeps Emberhut out of, since when. */
export const framing = new Map<number, number>();
/** Why Chrome last detached a tab, for the server's errors. */
export const dropped = new Map<number, string>();
/** Each tab's pill world in its top frame (its execution context id), and that frame's id. */
export const pillWorld = new Map<number, number>();
/** Every context of Emberhut's own world seen in each tab: their events never go back to the server. */
export const ownContexts = new Map<number, Set<number>>();

/** Notes a context of Emberhut's own world in `tabId`. */
export function noteOwnContext(tabId: number, contextId: number): void {
  const known = ownContexts.get(tabId) ?? new Set<number>();
  known.add(contextId);
  ownContexts.set(tabId, known);
}
export const mainFrame = new Map<number, string>();

/** How many times Emberhut let go of all its tabs: an attach still on its way when one came lets its tab go at once. */
export const counters = { letGoes: 0 };

/** Forgets a tab that closed. */
export function forget(tabId: number): void {
  for (const set of [attached, adopted, made, joining]) set.delete(tabId);
  for (const map of [closing, urls, framing, dropped, pillWorld, mainFrame, ownContexts]) map.delete(tabId);
}
