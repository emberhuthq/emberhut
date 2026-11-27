import type { EmberhutHttp } from "../http";
import type {
  Branches,
  FolderListing,
  FolderRoots,
  NewSessionRequest,
  NewSessionResult,
  Reply,
  RunnerMachine,
  RunnerSession,
  SessionHead,
  SessionSendResult,
  SessionSettings,
  SessionTranscript,
  ExternalThreadActivity,
} from "../types";
import { agentStep } from "../agentSteps";
import { EXTERNAL_THREAD_ACTIVITY_PATH } from "../wire";
import { fileForSend } from "./chat";

const base = (id: string) => `/api/runner/sessions/${encodeURIComponent(id)}`;

export function sessionEndpoints(http: EmberhutHttp) {
  return {
    async list(filter: { task?: number; room?: number; direct?: boolean } = {}): Promise<RunnerSession[]> {
      const page = await http.get<{ sessions: RunnerSession[] }>("/api/runner/sessions", {
        task: filter.task,
        room: filter.room,
        direct: filter.direct ? 1 : undefined,
      });
      return page.sessions;
    },
    /** One of your own sessions, on the machine and folder you picked. */
    start(request: NewSessionRequest): Promise<NewSessionResult> {
      return http.post<NewSessionResult>(
        "/api/runner/sessions",
        { ...request, files: request.files.map(fileForSend) },
        { timeoutMs: 60_000 },
      );
    },
    /**
     * Steps after `after` (0: the newest `limit`, 400 by default). `agent` (a sub-agent's call id) keeps only that agent's
     * own steps: the server narrows a worker's, and a runner session's are narrowed here (agentStep).
     */
    async transcript(id: string, page: { after: number; limit?: number; agent?: string }): Promise<SessionTranscript> {
      const transcript = await http.get<SessionTranscript>(`${base(id)}/transcript`, page, { timeoutMs: 30_000 });
      if (page.agent === undefined) return transcript;
      const events = transcript.events.flatMap((step) => agentStep(id, page.agent, step) ?? []);
      return { ...transcript, events };
    },
    send(id: string, text: string, files: NewSessionRequest["files"] = []): Promise<SessionSendResult> {
      return http.post<SessionSendResult>(`${base(id)}/send`, { text, files: files.map(fileForSend) }, { timeoutMs: 30_000 });
    },
    /** Stops the turn it is in. */
    interrupt(id: string): Promise<Reply & { running?: boolean }> {
      return http.post(`${base(id)}/interrupt`);
    },
    /** Stops it, its queued turns too. */
    stop(id: string): Promise<Reply & { running?: boolean }> {
      return http.post(`${base(id)}/stop`);
    },
    settings(id: string, settings: SessionSettings): Promise<Reply & { session: SessionHead }> {
      return http.post(`${base(id)}/settings`, settings);
    },
    /** Where to start picking a folder (no path), or one folder's folders. */
    folders<P extends string | undefined>(machine: RunnerMachine, path?: P): Promise<P extends string ? FolderListing : FolderRoots> {
      return http.get("/api/runner/folders", { machine, path }, { timeoutMs: 30_000 });
    },
    branches(machine: RunnerMachine, cwd: string): Promise<Branches> {
      return http.get<Branches>("/api/runner/branches", { machine, cwd }, { timeoutMs: 30_000 });
    },
    /** An external thread's activity, read-only. */
    externalThreadActivity(machine: string, thread: string): Promise<ExternalThreadActivity> {
      return http.get<ExternalThreadActivity>(EXTERNAL_THREAD_ACTIVITY_PATH, { machine, thread }, { timeoutMs: 30_000 });
    },
  };
}
