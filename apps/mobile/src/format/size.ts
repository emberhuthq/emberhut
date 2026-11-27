/** Bytes in a GB as the app says sizes (a gibibyte, as `free -h`, `df -h` and the runners count). */
export const bytesPerGb = 1024 ** 3;

/** Bytes as GB with one decimal: "12.3 GB". */
export function gb(bytes: number): string {
  return `${(bytes / bytesPerGb).toFixed(1)} GB`;
}

/** "10.2 of 62.0 GB". */
export function gbOf(used: number, total: number): string {
  return `${(used / bytesPerGb).toFixed(1)} of ${gb(total)}`;
}

/** A share 0…1 as a whole percent: "12%". */
export function percent(share: number): string {
  return `${Math.round(share * 100)}%`;
}

/** How full, 0…1, with nothing to divide by as empty. */
export function share(used: number, total: number): number {
  return total > 0 ? Math.min(Math.max(used / total, 0), 1) : 0;
}

/**
 * A file's size in binary units, as the desktop app says it: "512 B", "1.5 KiB", "12 KiB", "3.4 MiB", "1.2 GiB" (one
 * decimal under 10, halves rounded up).
 */
export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"] as const;
  let unit = 1024;
  let at = 0;
  while (at < units.length - 1 && bytes >= unit * 1024) {
    unit *= 1024;
    at++;
  }
  const tenths = Math.floor((bytes * 10 + unit / 2) / unit);
  if (tenths < 100) return `${Math.floor(tenths / 10)}.${tenths % 10} ${units[at]}`;
  return `${Math.floor((bytes + unit / 2) / unit)} ${units[at]}`;
}
