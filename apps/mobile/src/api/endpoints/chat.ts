import type { EmberhutHttp } from "../http";
import type { ChatFile, ChatPage, Reply, TalkRequest, TalkResult } from "../types";

export function chatEndpoints(http: EmberhutHttp) {
  return {
    /** The 60 messages before `before`, or the newest 60 after `after` (0: the newest page). */
    history(room: number, page: { before: number } | { after: number }): Promise<ChatPage> {
      return http.get<ChatPage>("/api/history", { room, ...page });
    },
    /** The whole file objects go back: the server only accepts files from its uploads folder. */
    talk(request: TalkRequest): Promise<TalkResult> {
      return http.post<TalkResult>("/api/talk", { ...request, files: request.files.map(fileForSend) }, { timeoutMs: 30_000 });
    },
    seen(room: number): Promise<Reply> {
      return http.post<Reply>("/api/seen", { room });
    },
    /** Starts or stops recording a call on the desktop app's machine; the transcript comes to `room`. */
    recordCall(action: "start" | "stop", room: number): Promise<Reply> {
      return http.post<Reply>("/api/calls/record", { action, room }, { timeoutMs: 40_000 });
    },
  };
}

/** A file as every send endpoint takes it back. */
export function fileForSend(file: ChatFile): Required<ChatFile> {
  return { name: file.name, url: file.url, kind: file.kind, path: file.path ?? "" };
}
