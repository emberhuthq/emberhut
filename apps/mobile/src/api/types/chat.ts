import type { Instant } from "./common";

/** A file stored on the server for the chat (POST /api/uploads). Sending one back needs its `path` unchanged. */
export interface ChatFile {
  name: string;
  /** A server path ("/uploads/…"), fetched with the session. */
  url: string;
  kind: "image" | "video" | "file";
  /** Where it sits on the server: the server only accepts files from its uploads folder. */
  path?: string;
}

/**
 * What an answer is about, shown as chips under it. A report or file points at /api/artifacts/<id> or an https link;
 * an HTML report is also a ChatFile ending in .html.
 */
export interface MessageRef {
  kind: "task" | "pr" | "ticket" | "card" | "report" | "file";
  label: string;
  title?: string;
  url?: string;
  task?: number;
  name?: string;
}

/** The message a reply quotes. */
export interface ChatQuote {
  id: number;
  mine: boolean;
  at?: Instant | null;
  text: string;
  file?: ChatFile | null;
}

export interface ChatMsg {
  /** The server's row id: stable however the list around it changes. */
  id: number;
  mine: boolean;
  /** The server's own line to Emberhut (events, check-ins), not Emberhut's words. */
  event?: boolean | null;
  text: string;
  /** A relative label ("now", "4m", "3h", "2d") as of when the server answered (docs/api.md); screens show `at` as a clock. */
  time: string;
  at?: Instant | null;
  files?: ChatFile[] | null;
  reply?: ChatQuote | null;
  refs?: MessageRef[] | null;
}

/** GET /api/history: at most 60 messages, oldest first. */
export interface ChatPage {
  chat: ChatMsg[];
}

/** One /api/live value while Emberhut works: its current step. `text` is always "" now; the answer arrives in the chat. */
export interface Live {
  text: string;
  doing: string | null;
}

/** POST /api/talk. */
export interface TalkRequest {
  text: string;
  files: ChatFile[];
  room: number;
  replyTo?: number;
}

/** The answer comes in the chat; a refusal arrives as an error. */
export interface TalkResult {
  ok: boolean;
  started: boolean;
  /** Family rooms: the message's id. */
  id?: number;
}
