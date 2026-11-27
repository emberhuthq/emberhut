// Self-update: the extension is loaded unpacked from one folder per computer, which the owner's server keeps in step
// (on a Mac, through the Mac helper). Told the folder holds a newer version, the extension reloads itself from it:
// never while it holds a tab, and only once per version. A reload that comes back still older (the profile loads
// the extension from another folder, or the folder wasn't what was said) is never tried again for that version, so
// nothing loops; a newer version is tried once in its turn.

/** Whether version `a` is newer than `b`, part by part (a missing part is 0). */
export function newer(a: string, b: string): boolean {
  const x = a.split(".").map(Number);
  const y = b.split(".").map(Number);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const own = x[i] || 0;
    const other = y[i] || 0;
    if (own !== other) return own > other;
  }
  return false;
}

/** The last reload tried: for which version, and when. */
export interface Tried {
  version: string;
  at: number;
}

/** Whether to reload now for `pending` (the folder's version), running `running` and holding `holding` tabs. */
export function shouldReload(pending: string | null, running: string, holding: number, tried: Tried | null): boolean {
  if (!pending || !newer(pending, running) || holding > 0) return false;
  return !(tried && !newer(pending, tried.version));
}

/** Whether a stored value is a tried reload. */
export function isTried(value: unknown): value is Tried {
  if (typeof value !== "object" || value === null) return false;
  const t = value as Record<string, unknown>;
  return typeof t.version === "string" && typeof t.at === "number";
}
