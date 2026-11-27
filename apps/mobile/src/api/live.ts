import { agentEvent } from "./agentSteps";
import { ApiError } from "./errors";
import type { Query } from "./fetch";
import type { EmberhutHttp } from "./http";
import { parseSse, type SseMessage } from "./sse";
import type { ApprovalsEvent, Live, PartyLiveEvent, SessionLiveEvent } from "./types";

export interface StreamOptions {
  signal: AbortSignal;
  /** No bytes for this long (the server pings every 15 s): the connection is dead, open a new one. */
  idleMs?: number;
}

/** One connection to a server-sent event stream, its messages until it ends or goes quiet. */
async function* openStream(http: EmberhutHttp, path: string, query: Query, options: StreamOptions): AsyncGenerator<SseMessage> {
  const idle = new AbortController();
  const abort = () => idle.abort();
  options.signal.addEventListener("abort", abort);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const alive = () => {
    clearTimeout(timer);
    timer = setTimeout(abort, options.idleMs ?? 45_000);
  };
  try {
    alive();
    const response = await http.send(path, { query, timeoutMs: 0, signal: idle.signal, headers: { Accept: "text/event-stream" } });
    if (!response.ok || response.body === null) throw new ApiError(response.status, `HTTP ${response.status}`);
    for await (const message of parseSse(response.body, { onChunk: alive, signal: idle.signal })) yield message;
  } finally {
    clearTimeout(timer);
    options.signal.removeEventListener("abort", abort);
    idle.abort();
  }
}

export interface FollowOptions {
  signal: AbortSignal;
  /** Between a dropped connection and the next. */
  retryMs?: number;
  idleMs?: number;
  /** Connected (true) or dropped (false). */
  onConnection?: (connected: boolean) => void;
}

/** Keeps a stream open until `signal` aborts, reconnecting after drops; `connect` opens one connection. */
async function follow(
  connect: (signal: AbortSignal) => AsyncIterable<SseMessage>,
  onMessage: (message: SseMessage) => void,
  options: FollowOptions,
): Promise<void> {
  const { signal } = options;
  while (!signal.aborted) {
    let connected = false;
    try {
      for await (const message of connect(signal)) {
        if (!connected) {
          connected = true;
          options.onConnection?.(true);
        }
        onMessage(message);
      }
    } catch {
      // dropped, refused or unreachable: try again shortly
    }
    if (connected) options.onConnection?.(false);
    if (signal.aborted) return;
    await sleep(options.retryMs ?? 2_000, signal);
  }
}

/** The room stream's event when an approval request changes; its data is an ApprovalsEvent. */
export const APPROVALS_EVENT = "approvals";

export interface RoomLiveOptions extends FollowOptions {
  /** An approval request changed (an `approvals` event): what shows approvals should be fetched again. */
  onApprovals?: (event: ApprovalsEvent) => void;
}

/**
 * A room's live line while Emberhut works (GET /api/live): a Live, then null when the turn is over, each an unnamed event.
 * A named event is something else: `approvals` goes to `onApprovals`, and a name this app doesn't know is left alone
 * (it would never read as a Live).
 */
export function followRoomLive(
  http: EmberhutHttp,
  room: number,
  onLive: (live: Live | null) => void,
  options: RoomLiveOptions,
): Promise<void> {
  return follow(
    (signal) => openStream(http, "/api/live", { room }, { signal, idleMs: options.idleMs }),
    (message) => {
      if (message.event === APPROVALS_EVENT) {
        const event = parseJson<ApprovalsEvent>(http, message.data);
        if (typeof event === "object" && event !== null && typeof event.id === "number") options.onApprovals?.(event);
        return;
      }
      if (message.event !== "message") return;
      const live = parseJson<Live | null>(http, message.data);
      if (live !== undefined) onLive(live);
    },
    options,
  );
}

/**
 * A runner session's live events (GET /api/runner/sessions/<id>/live). Each reconnect asks only for the steps after the last
 * one seen, so nothing comes twice. With `agent`, only that sub-agent's own steps reach `onEvent` (agentEvent).
 */
export function followSession(
  http: EmberhutHttp,
  id: string,
  from: { after: number; agent?: string },
  onEvent: (event: SessionLiveEvent) => void,
  options: FollowOptions,
): Promise<void> {
  let after = from.after;
  return follow(
    (signal) =>
      openStream(
        http,
        `/api/runner/sessions/${encodeURIComponent(id)}/live`,
        { after, agent: from.agent },
        { signal, idleMs: options.idleMs },
      ),
    (message) => {
      const event = parseJson<SessionLiveEvent>(http, message.data);
      if (event === undefined || event === null) return;
      if (event.type === "step") after = Math.max(after, event.step.seq);
      const shown = agentEvent(id, from.agent, event);
      if (shown !== null) onEvent(shown);
    },
    options,
  );
}

/** A party's live talk (GET /api/parties/<id>/live): a snapshot on each connection, then each change. */
export function followParty(
  http: EmberhutHttp,
  id: number,
  onEvent: (event: PartyLiveEvent) => void,
  options: FollowOptions,
): Promise<void> {
  return follow(
    (signal) => openStream(http, `/api/parties/${id}/live`, {}, { signal, idleMs: options.idleMs }),
    (message) => {
      const event = parseJson<PartyLiveEvent>(http, message.data);
      if (typeof event === "object" && event !== null && "type" in event) onEvent(event);
    },
    options,
  );
}

/** An event's data, in the app's words; undefined when it isn't JSON. */
function parseJson<T>(http: EmberhutHttp, data: string): T | undefined {
  try {
    return http.dialect.read(JSON.parse(data)) as T;
  } catch {
    return undefined;
  }
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(done, ms);
    signal.addEventListener("abort", done);
    function done() {
      clearTimeout(timer);
      signal.removeEventListener("abort", done);
      resolve();
    }
  });
}
