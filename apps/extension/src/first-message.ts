// The socket's first message. Pairing tells the server only the code (and this profile's old token, to replace it,
// only when that token is the same server's): a server being paired with may be a stranger until the owner's code
// proves it. What the profile is (its version, system and Chrome) goes only in the hello, to the server it is paired
// with.

import type { PairWith } from "./pairing";
import { PROTOCOL } from "./protocol";

/** What the profile says about itself in its hello. */
export interface About {
  version: string;
  os?: string | undefined;
  agent?: string | undefined;
}

/** The first message on a socket: `pair` for `pairing`, else `hello` with the profile's `token`. */
export function firstMessage(
  pairing: PairWith | null,
  saved: { server: string | null; token: string | null },
  about: About,
): { method: string; params: Record<string, unknown> } {
  if (pairing) {
    const old = saved.token && saved.server === pairing.server ? { old: saved.token } : {};
    return { method: "pair", params: { protocol: PROTOCOL, code: pairing.code, ...old } };
  }
  return { method: "hello", params: { protocol: PROTOCOL, token: saved.token, ...about } };
}
