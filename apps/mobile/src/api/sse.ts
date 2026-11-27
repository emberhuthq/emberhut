/** One server-sent event: its `event:` name ("message" when none), its data lines joined, its `id:`. */
export interface SseMessage {
  event: string;
  data: string;
  id?: string;
}

/**
 * Reads a text/event-stream body as messages, following the spec's line rules: LF, CRLF or CR line ends, `:` comments
 * (the server's pings), several `data:` lines joined with "\n", and a message only once its blank line arrives.
 * `onChunk` hears of every chunk read, pings included; `signal` ends the reading, whatever the fetch does with it.
 */
export async function* parseSse(
  body: ReadableStream<Uint8Array>,
  options: { onChunk?: () => void; signal?: AbortSignal } = {},
): AsyncGenerator<SseMessage> {
  const reader = body.getReader();
  const stop = () => void reader.cancel().catch(() => undefined);
  if (options.signal?.aborted) stop();
  options.signal?.addEventListener("abort", stop);
  const decoder = new TextDecoder();
  let buffer = "";
  let event = "";
  let data: string[] = [];
  let id: string | undefined;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (!done) options.onChunk?.();
      buffer += done ? decoder.decode() : decoder.decode(value, { stream: true });
      for (;;) {
        const end = buffer.search(/\r\n|\r|\n/);
        // A lone CR at the end may be the first half of a CRLF still on its way.
        if (end < 0 || (end === buffer.length - 1 && buffer[end] === "\r" && !done)) break;
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + (buffer.startsWith("\r\n", end) ? 2 : 1));
        if (line === "") {
          if (data.length > 0) yield { event: event || "message", data: data.join("\n"), ...(id === undefined ? {} : { id }) };
          event = "";
          data = [];
          continue;
        }
        if (line.startsWith(":")) continue;
        const colon = line.indexOf(":");
        const field = colon < 0 ? line : line.slice(0, colon);
        const raw = colon < 0 ? "" : line.slice(colon + 1);
        const value = raw.startsWith(" ") ? raw.slice(1) : raw;
        if (field === "data") data.push(value);
        else if (field === "event") event = value;
        else if (field === "id") id = value;
      }
      if (done) return;
    }
  } finally {
    options.signal?.removeEventListener("abort", stop);
    reader.releaseLock();
  }
}
