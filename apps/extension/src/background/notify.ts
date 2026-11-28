// Chrome notifications are optional: the owner turns them on in the popup (Chrome asks then), and without them the
// badge, the popup, the pill on the page and the Emberhut app say the same things. Without the permission Chrome
// gives the extension no `chrome.notifications` at all, so every use goes through here.

const WANTED: chrome.permissions.Permissions = { permissions: ["notifications"] };

/** Whether the owner turned Chrome notifications on. */
export function canNotify(): Promise<boolean> {
  return chrome.permissions.contains(WANTED);
}

/** Shows a notification when they're on; nothing otherwise. */
export async function notify(id: string, options: chrome.notifications.NotificationCreateOptions): Promise<void> {
  if (await canNotify()) await chrome.notifications.create(id, options);
}

/** Takes a notification down, when they're on. */
export async function unnotify(id: string): Promise<void> {
  if (await canNotify()) await chrome.notifications.clear(id);
}
