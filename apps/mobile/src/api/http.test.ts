import { describe, expect, it } from "vitest";
import { ApiError, CONTRACT_TASK_KEY_PREFIX, isServerPath, taskKey, type AuthEvent } from "@/api";
import { fakeApi, json, memoryStorage, tokens } from "@/test/fakes";

describe("EmberhutHttp", () => {
  it("builds GETs with the query, leaving out empty values", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/tasks", () => json(200, { tasks: [] }));
    await api.tasks.list({ q: taskKey(212), project: "none", status: ["waiting", "todo"], closed: true, owner: undefined });
    const url = server.to("/api/tasks")[0]!.url;
    // The search box's key goes out in the server's spelling, whatever prefix the build shows.
    expect(Object.fromEntries(url.searchParams)).toEqual({
      q: `${CONTRACT_TASK_KEY_PREFIX}212`,
      project: "none",
      status: "waiting,todo",
      closed: "1",
    });
    expect(server.sent[0]!.headers.authorization).toBeUndefined();
  });

  it("sends JSON bodies as POSTs with the session", async () => {
    const storage = memoryStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "i" });
    const { server, clock, api } = fakeApi({ storage });
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(1, clock.now)));
    server.on("POST", "/api/talk", () => json(200, { ok: true, started: true }));
    const file = { name: "a.png", url: "/uploads/1-a.png", kind: "image" as const };

    expect(await api.chat.talk({ text: "hi", files: [file], room: 3, replyTo: 41 })).toEqual({ ok: true, started: true });
    const talk = server.to("/api/talk")[0]!;
    expect(talk.method).toBe("POST");
    expect(talk.headers["content-type"]).toBe("application/json");
    expect(talk.headers.authorization).toBe("Bearer access-1");
    expect(talk.body).toEqual({ text: "hi", files: [{ ...file, path: "" }], room: 3, replyTo: 41 });
  });

  it("uploads a file from disk as the raw body, with the name in the address", async () => {
    const storage = memoryStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "i" });
    const { server, clock, api } = fakeApi({ storage });
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(1, clock.now)));
    const stored = { name: "note.pdf", url: "/uploads/ab-note.pdf", kind: "file", path: "/home/ada/.emberhut/uploads/ab-note.pdf" };
    server.on("POST", "/api/uploads", () => json(200, { ok: true, file: stored }));
    expect(await api.files.upload("file:///cache/note.pdf", "note.pdf")).toEqual(stored);
    const upload = server.to("/api/uploads")[0]!;
    expect(upload.url.searchParams.get("name")).toBe("note.pdf");
    expect(upload.headers["content-type"]).toBe("application/octet-stream");
    expect(upload.headers.authorization).toBe("Bearer access-1");
    expect(upload.body).toBe("file:///cache/note.pdf");
  });

  it("throws the server's reason for an error status and for ok: false", async () => {
    const { server, api } = fakeApi();
    server.on("POST", "/api/settings", () => json(400, { ok: false, error: "no such task" }));
    await expect(api.settings.apply({ taskDecide: 9, choice: "Yes" })).rejects.toMatchObject({ status: 400, message: "no such task" });
    server.on("POST", "/api/seen", () => json(200, { ok: false, error: "no such room" }));
    await expect(api.chat.seen(99)).rejects.toMatchObject({ status: 200, message: "no such room" });
  });

  it("calls a network failure unreachable", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/facts", () => {
      throw new TypeError("Network request failed");
    });
    const error = await api.settings.facts().catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).unreachable).toBe(true);
  });

  it("tells a missing session from a device the server doesn't let in", async () => {
    const { server, auth, api } = fakeApi();
    const events: AuthEvent[] = [];
    auth.subscribe((e) => events.push(e));

    server.on("GET", "/api/facts", () => json(401, { error: "Sign in with Apple first.", signIn: true }));
    await expect(api.settings.facts()).rejects.toMatchObject({ status: 401 });
    expect(events).toEqual([{ kind: "signedOut", reason: "required" }]);

    server.on("GET", "/api/facts", () => json(401, { error: "not Emberhut's device" }));
    await expect(api.settings.facts()).rejects.toMatchObject({ status: 401, message: "not Emberhut's device" });
    await expect(api.settings.facts()).rejects.toMatchObject({ status: 401 });
    server.on("GET", "/api/facts", () => json(200, { facts: [] }));
    await api.settings.facts();
    expect(events.slice(1)).toEqual([{ kind: "refused" }, { kind: "reachable" }]);
  });

  it("renews instead of signing out when a held session's access token was stale", async () => {
    const storage = memoryStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "i" });
    const { server, clock, auth, api } = fakeApi({ storage });
    const events: AuthEvent[] = [];
    auth.subscribe((e) => events.push(e));
    let n = 0;
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(++n, clock.now)));
    let stale = true;
    server.on("GET", "/api/facts", () => (stale ? json(401, { signIn: true }) : json(200, { facts: [] })));

    await expect(api.settings.facts()).rejects.toMatchObject({ status: 401 });
    stale = false;
    await api.settings.facts();
    expect(server.to("/api/facts")[1]!.headers.authorization).toBe("Bearer access-2");
    expect(events).toEqual([]);
  });

  it("puts the session in file URLs for players and in headers for images", async () => {
    const storage = memoryStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "i" });
    const { server, clock, auth, api } = fakeApi({ storage });
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(1, clock.now)));
    await auth.accessToken();
    expect(api.files.url("/api/artifacts/66")).toBe("http://emberhut.test:18990/api/artifacts/66?token=access-1");
    expect(api.files.source("/uploads/x.png")).toEqual({
      uri: "http://emberhut.test:18990/uploads/x.png",
      headers: { Authorization: "Bearer access-1" },
    });
  });
});

describe("the session stays on Emberhut's server", () => {
  const away = [
    "//evil.example/a.png",
    "/\\evil.example/a.png",
    "/\t/evil.example/a.png",
    "/\n\\evil.example/a.png",
    "https://evil.example/a.png",
    "evil.example/a.png",
    " //evil.example",
  ];

  it("never builds a file source, a file address or a request for another server", async () => {
    const storage = memoryStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "i" });
    const { server, clock, api } = fakeApi({ storage });
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(1, clock.now)));
    await api.http.get("/api/auth/status").catch(() => undefined);
    for (const path of away) {
      // What a screen checks before it builds one agrees, so a picture's path can't throw while it renders.
      expect(isServerPath(path), path).toBe(false);
      expect(() => api.files.source(path), path).toThrow(/Not a path on Emberhut's server/);
      expect(() => api.files.url(path), path).toThrow(/Not a path on Emberhut's server/);
      await expect(api.files.bytes(path), path).rejects.toThrow(/Not a path on Emberhut's server/);
    }
    expect(server.sent.every((r) => r.url.host === "emberhut.test:18990")).toBe(true);
    expect(api.files.source("/uploads/a.png")).toEqual({
      uri: "http://emberhut.test:18990/uploads/a.png",
      headers: { Authorization: "Bearer access-1" },
    });
  });
});
