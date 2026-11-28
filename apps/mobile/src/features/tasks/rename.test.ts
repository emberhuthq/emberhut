import { describe, expect, it } from "vitest";
import { ASSISTANT, TASK_PATH, taskKey, type Issue } from "@/api";
import { fakeApi, json } from "@/test/fakes";
import { newTitle, renameFailure } from "./rename";

const task: Issue = { id: 4, key: taskKey(4), title: "Bake rye", owner: ASSISTANT, status: "todo", priority: 0, today: false, updated: "" };

describe("Rename", () => {
  it("sends the title typed, trimmed, unless it is the task's", () => {
    expect(newTitle("Bake", "  Bake rye ")).toBe("Bake rye");
    expect(newTitle("Bake", " Bake ")).toBeNull();
    expect(newTitle("Bake", "   ")).toBe("");
  });

  it("patches the task's path with its title and reads the task back", async () => {
    const { server, api } = fakeApi();
    const path = `${TASK_PATH}4`;
    server.on("PATCH", path, () => json(200, task));
    await expect(api.tasks.rename(4, "Bake rye")).resolves.toMatchObject({ id: 4, title: "Bake rye" });
    expect(server.to(path)[0]?.body).toEqual({ title: "Bake rye" });
  });

  it("says the server's reason when it refuses: an empty or too long title, an unknown task", async () => {
    const { server, api } = fakeApi();
    server.on("PATCH", `${TASK_PATH}4`, () => json(400, { ok: false, error: "a title is 1 to 200 characters" }));
    server.on("PATCH", `${TASK_PATH}9`, () => json(404, { ok: false, error: "no such task" }));
    const empty = await api.tasks.rename(4, "").catch((cause: unknown) => cause);
    const unknown = await api.tasks.rename(9, "x").catch((cause: unknown) => cause);
    expect(empty).toMatchObject({ status: 400 });
    expect(renameFailure(empty)).toBe("Not renamed: a title is 1 to 200 characters");
    expect(unknown).toMatchObject({ status: 404 });
    expect(renameFailure(unknown)).toBe("Not renamed: no such task");
    expect(renameFailure(null)).toBe("Not renamed: unknown");
  });
});
