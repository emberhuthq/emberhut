import { describe, expect, it } from "vitest";
import { ASSISTANT, SESSION_TASK_ROUTE, taskKey, type Issue, type TaskStatus } from "@/api";
import { fakeApi, json } from "@/test/fakes";
import { isMovable, moveFailure, moveTargets } from "./moveToTask";

function issue(id: number, status: TaskStatus = "todo", title = `Task ${id}`): Issue {
  return { id, key: taskKey(id), title, owner: ASSISTANT, status, priority: 0, today: false, updated: "" };
}

describe("Move to task…", () => {
  it("is offered on a runner's and a worker's sessions only", () => {
    expect(isMovable("r-1a2b")).toBe(true);
    expect(isMovable("w-12")).toBe(true);
    expect(isMovable("t3-9")).toBe(false);
    expect(isMovable("r-")).toBe(false);
    expect(isMovable("")).toBe(false);
  });

  it("lists the open tasks, without the session's own, those put away here or the same one twice", () => {
    const tasks = [
      issue(1),
      issue(2, "working"),
      issue(3, "waiting"),
      issue(4, "done"),
      issue(5, "canceled"),
      issue(6),
      issue(2, "working"),
    ];
    expect(moveTargets(tasks, [1], new Set([6])).map((t) => t.id)).toEqual([2, 3]);
  });

  it("finds a task by its key or by words of its title, in any case", () => {
    const tasks = [issue(12, "todo", "Ship the beta"), issue(212, "todo", "Fix the login"), issue(30, "todo", "Write the notes")];
    expect(moveTargets(tasks, [], new Set(), "  BETA ").map((t) => t.id)).toEqual([12]);
    expect(moveTargets(tasks, [], new Set(), taskKey(212).toLowerCase()).map((t) => t.id)).toEqual([212]);
    expect(moveTargets(tasks, [], new Set(), "the").map((t) => t.id)).toEqual([12, 212, 30]);
    expect(moveTargets(tasks, [], new Set(), "nothing like it")).toEqual([]);
  });

  it("posts the task to the session's route and reads the task it works on now", async () => {
    const { server, api } = fakeApi();
    const path = `/api/runner/sessions/r-1a2b/${SESSION_TASK_ROUTE}`;
    server.on("POST", path, () => json(200, { ok: true, task: 12 }));
    await expect(api.sessions.moveToTask("r-1a2b", 12)).resolves.toEqual({ ok: true, task: 12 });
    expect(server.to(path)[0]?.body).toEqual({ task: 12 });
  });

  it("says the server's reason when it refuses: a closed task, an unknown session or task", async () => {
    const { server, api } = fakeApi();
    server.on("POST", `/api/runner/sessions/r-1/${SESSION_TASK_ROUTE}`, () => json(400, { ok: false, error: "T-4 is done" }));
    server.on("POST", `/api/runner/sessions/r-2/${SESSION_TASK_ROUTE}`, () => json(404, { ok: false, error: "no such session" }));
    const closed = await api.sessions.moveToTask("r-1", 4).catch((cause: unknown) => cause);
    const unknown = await api.sessions.moveToTask("r-2", 4).catch((cause: unknown) => cause);
    expect(closed).toMatchObject({ status: 400 });
    expect(moveFailure(closed)).toBe("Not moved: T-4 is done");
    expect(unknown).toMatchObject({ status: 404 });
    expect(moveFailure(unknown)).toBe("Not moved: no such session");
    expect(moveFailure("?")).toBe("Not moved: unknown");
  });
});
