/** A server's address as Settings and the banner name it: its host and port. */
export function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}
