// Pairing: the owner asks the app for a link (…/chrome/pair#CODE on their server's address) and opens it in the
// profile; or pastes the link, or the code and the address, in the popup. The server's address comes only from
// pairing: there is no built-in one.

/** A pairing: the server's address (its origin, or origin and path, no final slash) and the code. */
export interface PairWith {
  server: string;
  code: string;
}

/** A code as the server shows it (`ABCD-EFGH`), from what was typed or linked; null for one that isn't. */
export function codeOf(text: string): string | null {
  const raw = text.trim().toUpperCase().replace(/[\s-]/g, "");
  return /^[A-Z0-9]{8}$/.test(raw) ? `${raw.slice(0, 4)}-${raw.slice(4)}` : null;
}

/** A server's address as kept: http or https, without a final slash, query or fragment; null for one that isn't. */
export function serverOf(text: string): string | null {
  let u: URL;
  try {
    u = new URL(text.trim());
  } catch {
    return null;
  }
  if (!/^https?:$/.test(u.protocol) || !u.hostname || u.username || u.password) return null;
  return `${u.origin}${u.pathname}`.replace(/\/+$/, "");
}

/** The pairing a tab's address is, when it is a pairing link (`<server>/chrome/pair#CODE`); null otherwise. */
export function linkOf(url: string): PairWith | null {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return null;
  }
  if (!u.pathname.endsWith("/chrome/pair") || !u.hash) return null;
  let hash: string;
  try {
    hash = decodeURIComponent(u.hash.slice(1));
  } catch {
    return null;
  }
  const code = codeOf(hash);
  const server = serverOf(`${u.origin}${u.pathname.slice(0, -"/chrome/pair".length)}`);
  return code && server ? { server, code } : null;
}

/** What the popup's two boxes say: a whole link in the first, or a code there and the address in the second. */
export function readPairing(codeOrLink: string, address: string): PairWith | string {
  const link = linkOf(codeOrLink.trim());
  if (link) return link;
  const code = codeOf(codeOrLink);
  if (!code) return "Paste the pairing link from the Emberhut app, or its code (ABCD-EFGH)";
  const server = serverOf(address);
  if (!server) return "The server's address, as https://… (it is the start of the pairing link)";
  return { server, code };
}
