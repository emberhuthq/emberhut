// The socket's messages through the build's wire dialect (src/dialect.ts). Every message's name is rewritten, both ways;
// the params only of the protocol's own named messages (src/protocol.ts), and never their `token`, which is opaque. A
// relayed call's arguments, its answer and the browser's events carry the pages' own text and addresses, which go as
// they are; so does the first message (`pair` or `hello`) past its name.

import type { Dialect } from "./dialect";
import { INITIALIZED, Msg } from "./protocol";

/** A message on the socket, either way. */
export interface Message {
  id?: unknown;
  method?: unknown;
  params?: unknown;
  [other: string]: unknown;
}

const NAMED: ReadonlySet<string> = new Set([...Object.values(Msg), INITIALIZED]);

/** `rewritten` with `given`'s token, when `given` has one. */
function keepToken(given: unknown, rewritten: unknown): unknown {
  if (typeof given !== "object" || given === null || !("token" in given)) return rewritten;
  if (typeof rewritten !== "object" || rewritten === null || Array.isArray(rewritten)) return rewritten;
  return { ...rewritten, token: given.token };
}

/** A message from the server, in the extension's words. */
export function readMessage(words: Dialect, message: Message): Message {
  if (words.identity || typeof message.method !== "string") return message;
  const method = words.fromServer(message.method);
  if (!NAMED.has(method) || message.id !== undefined || !("params" in message)) return { ...message, method };
  return { ...message, method, params: keepToken(message.params, words.jsonFromServer(message.params)) };
}

/** A message for the server, in its words. */
export function writeMessage(words: Dialect, message: Message): Message {
  if (words.identity || typeof message.method !== "string") return message;
  const method = words.toServer(message.method);
  if (!NAMED.has(message.method) || message.id !== undefined || !("params" in message)) return { ...message, method };
  return { ...message, method, params: keepToken(message.params, words.jsonToServer(message.params)) };
}
