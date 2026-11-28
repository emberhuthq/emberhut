import type { EmberhutHttp } from "../http";
import type { Issue, TaskMessageRequest, TaskMessageResult, TaskPage, TasksPage, TasksQuery } from "../types";
import { fileForSend } from "./chat";

export function taskEndpoints(http: EmberhutHttp) {
  return {
    /** Tasks from the whole list, done and archived ones too, newest first. */
    async list(query: TasksQuery = {}): Promise<Issue[]> {
      const page = await http.get<TasksPage>("/api/tasks", {
        q: query.q ?? "",
        limit: query.limit,
        project: query.project,
        closed: query.closed ? 1 : undefined,
        archived: query.archived ? 1 : undefined,
        area: query.area,
        status: query.status?.join(","),
        owner: query.owner,
      });
      return page.tasks;
    },
    /** One task with its history, subtasks, findings, links, runs, sessions, ultra run and plan. */
    get(id: number, signal?: AbortSignal): Promise<TaskPage> {
      return http.get<TaskPage>(`/api/tasks/${id}`, undefined, { signal });
    },
    /** Your reply on a task's page: it goes to the task (its worker, its session or Emberhut in its room), not the chat. */
    message(id: number, request: TaskMessageRequest): Promise<TaskMessageResult> {
      return http.post<TaskMessageResult>(
        `/api/tasks/${id}/message`,
        { text: request.text, files: request.files.map(fileForSend) },
        { timeoutMs: 30_000 },
      );
    },
  };
}
