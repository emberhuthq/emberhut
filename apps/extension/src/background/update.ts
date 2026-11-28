// Loaded unpacked (the developer's way, ADR 0023), the extension reloads itself from its folder once the server says
// the folder holds a newer version (emberhut.update): never while it holds a tab, and only once per version
// (version.ts). Installed from the Chrome Web Store, Chrome updates it and the server's word is ignored: the store
// adds an update_url to the manifest it serves.

import { shouldReload } from "../version";
import { attached } from "./held";
import { setSettings, settings } from "./state";

const VERSION = chrome.runtime.getManifest().version;
const FROM_STORE = typeof chrome.runtime.getManifest().update_url === "string";

/** The server said the folder holds `version`: reload when it's newer and nothing holds us. */
export async function noteUpdate(version: unknown): Promise<void> {
  if (FROM_STORE || typeof version !== "string" || version === "") return;
  await setSettings({ update: version });
  await maybeReload();
}

/** Reloads from the folder when an update waits and no tab is held; forgets an update this version already is. */
export async function maybeReload(): Promise<void> {
  const { update, tried } = await settings();
  if (!update) return;
  const now = Date.now();
  if (!shouldReload(update, VERSION, attached.size, tried)) {
    if (update === VERSION) await setSettings({ update: null });
    return;
  }
  await setSettings({ tried: { version: update, at: now } });
  chrome.runtime.reload();
}
