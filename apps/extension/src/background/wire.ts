// The socket to the server, as the rest of the worker uses it: whether it's ready (the server said hello) and how to
// tell the server something, in the server's words.

import { createDialect, parseDialect } from "../dialect";
import { writeMessage, type Message } from "../messages";

/** The dialect file's JSON, injected by scripts/build.ts: empty when the build has none. */
declare const EMBERHUT_WIRE_DIALECT: string;

/** This build's wire dialect (src/dialect.ts): without one, the extension's own words, unchanged. */
export const dialect = createDialect(parseDialect(EMBERHUT_WIRE_DIALECT));

/** `message` as the server spells it, as JSON. */
export function serverJson(message: Message): string {
  return JSON.stringify(writeMessage(dialect, message));
}

/** One socket and what the back-off knows of it. */
export interface Link {
  ws: WebSocket;
  since: number;
  /** Pings unanswered in a row. */
  missed: number;
  /** The server answered a ping at least once. */
  answers: boolean;
  /** The server said hello: the profile is known. */
  ready: boolean;
}

let current: Link | null = null;

/** The socket now, if any. */
export function link(): Link | null {
  return current;
}

/** Makes `next` the socket. */
export function setLink(next: Link | null): void {
  current = next;
}

/** Whether the server knows this profile now. */
export function ready(): boolean {
  return current?.ready === true;
}

/** Tells the server `method` (when it's listening). */
export function emit(method: string, params: unknown): void {
  if (current?.ready) current.ws.send(serverJson({ method, params }));
}
