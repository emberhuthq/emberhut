// What the socket to the hut may ride on. The pages Emberhut works in go to the hut over it, so it must be encrypted:
// https (wss), or plain http only where something else encrypts it or nothing leaves the computer: this computer
// itself (loopback), or a Tailscale address (100.64.0.0/10, fd7a:115c:a1e0::/48), whose WireGuard tunnel encrypts it.

/** Whether a plain-http hut at `host` (a URL's hostname) is still encrypted end to end, or never leaves the computer. */
export function plainAllowed(host: string): boolean {
  const h = host.toLowerCase().replace(/\.$/, "");
  if (h === "localhost" || h.endsWith(".localhost") || h === "[::1]") return true;
  const v4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(h);
  if (v4) {
    const [a, b] = [Number(v4[1]), Number(v4[2])];
    return a === 127 || (a === 100 && b >= 64 && b <= 127);
  }
  return h.startsWith("[fd7a:115c:a1e0:");
}

/** Why the extension won't pair with `server` (an address as pairing.ts keeps it), or null when it will. */
export function transportProblem(server: string): string | null {
  let u: URL;
  try {
    u = new URL(server);
  } catch {
    return "That isn't a hut's address";
  }
  if (u.protocol === "https:") return null;
  if (u.protocol === "http:" && plainAllowed(u.hostname)) return null;
  return `Emberhut pairs only over an encrypted link: give your hut an https:// address (or use its address on this computer or your Tailscale network), not ${u.origin}`;
}
