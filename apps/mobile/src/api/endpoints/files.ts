import { ApiError } from "../errors";
import type { EmberhutHttp } from "../http";
import type { ChatFile } from "../types";

export function fileEndpoints(http: EmberhutHttp) {
  return {
    /** POST /api/uploads: the file on the device (its URI) as the raw body, the name in the address. */
    async upload(file: string, name: string): Promise<ChatFile> {
      const reply = await http.json<{ ok?: boolean; file?: ChatFile }>("/api/uploads", { query: { name }, file, timeoutMs: 600_000 });
      if (!reply.file) throw new ApiError(200, "Upload not stored");
      return reply.file;
    },
    /** Speech to text through the server (it holds the key); `words` helps it spell names. */
    async transcribe(audio: string, name: string, words: string[] = []): Promise<string> {
      const reply = await http.json<{ text?: string }>("/api/transcribe", {
        query: { name, words: words.join(", ") },
        file: audio,
        timeoutMs: 60_000,
      });
      return reply.text ?? "";
    },
    /** A server file's bytes (/uploads/…, /api/artifacts/<id>, …/thumb). */
    async bytes(path: string): Promise<ArrayBuffer> {
      const response = await http.send(path, { timeoutMs: 60_000 });
      if (!response.ok) throw new ApiError(response.status, `HTTP ${response.status}`);
      return response.arrayBuffer();
    },
    /** A URL a player streams by itself, the session in its address. */
    url: (path: string) => http.fileUrl(path),
    /** An image source with the session in its header. */
    source: (path: string) => http.fileSource(path),
  };
}
