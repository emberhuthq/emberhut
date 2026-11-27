import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  createDialect,
  dialect,
  followRoomLive,
  fromServer,
  identityDialect,
  jsonFromServer,
  jsonToServer,
  LOOKING_HEADER,
  OWNER,
  parseDialect,
  taskKey,
  toServer,
  type EmberhutState,
  type Live,
} from "@/api";
import { fakeApi, json, memoryStorage, streamBody, tokens } from "@/test/fakes";

// A made-up server dialect: it calls the owner "person", starts task keys with "K-", links with other://, names the
// assistant's account field "helperAccount" and the looking header "X-Other-Looking".
const words = createDialect(
  parseDialect(
    JSON.stringify({
      words: [
        ["person", "owner"],
        ["Waiting on the person", "Waiting on the owner"],
      ],
      keys: [["helperAccount", "assistantAccount"]],
      headers: [["X-Other-Looking", "X-Emberhut-Looking"]],
      prefixes: [
        ["K-", "T-"],
        ["other://", "emberhut://"],
      ],
    }),
  ),
);

describe("parseDialect", () => {
  it("is empty, and rewrites nothing, when unset or blank", () => {
    for (const unset of [undefined, "", "  \n"]) {
      const dialect = createDialect(parseDialect(unset));
      expect(dialect).toBe(identityDialect);
      const value = { owner: "person", text: "K-1" };
      expect(dialect.read(value)).toBe(value);
    }
    expect(createDialect(parseDialect("{}")).identity).toBe(true);
  });

  it("refuses a malformed dialect, naming what's wrong", () => {
    expect(() => parseDialect("{")).toThrow("isn't JSON");
    expect(() => parseDialect("[]")).toThrow("isn't a JSON object");
    expect(() => parseDialect('{"names":[]}')).toThrow("unknown section: names");
    expect(() => parseDialect('{"words":[["a"]]}')).toThrow("words isn't a list");
    expect(() => parseDialect('{"words":[["a",""]]}')).toThrow("words isn't a list");
    expect(() => parseDialect('{"keys":{"a":"b"}}')).toThrow("keys isn't a list");
  });
});

// The cases the desktop client and the extension answer too.
interface Cases {
  dialect: unknown;
  texts: Record<"read" | "write", [string, string][]>;
  documents: Record<"read" | "write", [unknown, unknown][]>;
}
const cases = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../../../crates/emberhut-client/tests/fixtures/dialect.json", import.meta.url)), "utf8"),
) as Cases;

describe("the shared cases", () => {
  const shared = createDialect(parseDialect(JSON.stringify(cases.dialect)));
  it.each(cases.texts.read)("read %j as %j", (given, expected) => expect(shared.readText(given)).toBe(expected));
  it.each(cases.texts.write)("write %j as %j", (given, expected) => expect(shared.writeText(given)).toBe(expected));
  it.each(cases.documents.read)("read the document %j", (given, expected) => expect(shared.read(given)).toEqual(expected));
  it.each(cases.documents.write)("write the document %j", (given, expected) => expect(shared.write(given)).toEqual(expected));
});

describe("this build's dialect", () => {
  it.skipIf(!dialect.identity)("is empty unless the build sets one, and then its helpers give back what they're given", () => {
    const value = { owner: "person", text: "K-1" };
    expect(jsonFromServer(value)).toBe(value);
    expect(jsonToServer(value)).toBe(value);
    expect(fromServer("other://task/1")).toBe("other://task/1");
    expect(toServer("T-1")).toBe("T-1");
  });
});

describe("a dialect", () => {
  it("reads whole words, keys and prefixes into the app's words", () => {
    const answer = {
      owner: "person",
      week: { person: 2, assistant: 1 },
      helperAccount: { on: "a-1" },
      history: [{ who: "person", text: "Waiting on the person" }],
      action: "One yes on K-12, then K-13 (not SK-14)",
      token: "a1-K-2:K-3_K-4.K-5/K-6",
      link: "other://task/12",
      body: "See other://task/3 or [K-3](other://task/3)",
      persona: "personal",
      count: 3,
      done: null,
    };
    expect(words.read(answer)).toEqual({
      owner: "owner",
      week: { owner: 2, assistant: 1 },
      assistantAccount: { on: "a-1" },
      history: [{ who: "owner", text: "Waiting on the owner" }],
      action: "One yes on T-12, then T-13 (not SK-14)",
      token: "a1-K-2:K-3_K-4.K-5/K-6",
      link: "emberhut://task/12",
      body: "See emberhut://task/3 or [T-3](emberhut://task/3)",
      persona: "personal",
      count: 3,
      done: null,
    });
  });

  it("writes them back in the server's", () => {
    expect(words.write({ to: OWNER, q: taskKey(4), assistantAccount: "a-1", text: "a T-shirt" })).toEqual({
      to: "person",
      q: "K-4",
      helperAccount: "a-1",
      text: "a T-shirt",
    });
    expect(words.writeText("emberhut://task/1")).toBe("other://task/1");
    expect(words.readText("other://chat?room=4")).toBe("emberhut://chat?room=4");
    expect(words.writeKey("assistantAccount")).toBe("helperAccount");
  });

  it("keeps a request's field names that are words of the app's", () => {
    expect(words.write({ owner: OWNER })).toEqual({ owner: "person" });
    expect(words.writeKey("owner")).toBe("owner");
    expect(words.read({ person: 1 })).toEqual({ owner: 1 });
  });

  it("renames headers in any case", () => {
    expect(words.writeHeader(LOOKING_HEADER)).toBe("X-Other-Looking");
    expect(words.writeHeader("x-emberhut-looking")).toBe("X-Other-Looking");
    expect(words.readHeader("x-other-looking")).toBe("X-Emberhut-Looking");
    expect(words.writeHeader("If-None-Match")).toBe("If-None-Match");
  });
});

describe("the API client, built with a dialect", () => {
  it("writes the query, the headers and the body in the server's words, and reads the answer in the app's", async () => {
    const { server, api } = fakeApi({ dialect: words });
    server.on("GET", "/api/tasks", () => json(200, { tasks: [{ id: 4, key: "K-4", owner: "person", title: "Ship" }] }));
    const tasks = await api.tasks.list({ q: taskKey(4), owner: OWNER });
    expect(Object.fromEntries(server.to("/api/tasks")[0]!.url.searchParams)).toEqual({ q: "K-4", owner: "person" });
    expect(tasks).toEqual([{ id: 4, key: "T-4", owner: "owner", title: "Ship" }]);

    server.on("POST", "/api/tasks/4/message", () => json(200, { ok: true }));
    await api.tasks.message(4, { text: "about T-3", files: [] });
    expect(server.to("/api/tasks/4/message")[0]!.body).toEqual({ text: "about K-3", files: [] });

    server.on("GET", "/api/state", () => json(200, { room: 1, chat: [], helperAccount: { on: "a-1" } }));
    const state: EmberhutState = await api.state.get({ room: 1, looking: true });
    expect(server.to("/api/state")[0]!.headers["x-other-looking"]).toBe("1");
    expect(state.assistantAccount).toEqual({ on: "a-1" });
  });

  it("reads a live stream's events in the app's words", async () => {
    const { server, http } = fakeApi({ dialect: words });
    const stream = streamBody();
    server.on("GET", "/api/live", () => new Response(stream.body, { status: 200, headers: { "Content-Type": "text/event-stream" } }));
    const seen: (Live | null)[] = [];
    const controller = new AbortController();
    const done = followRoomLive(http, 1, (live) => seen.push(live), { signal: controller.signal, retryMs: 5 });
    await until(() => server.to("/api/live").length === 1);
    stream.write('data: {"text":"","doing":"Working on K-7"}\n\n');
    await until(() => seen.length === 1);
    controller.abort();
    await done;
    expect(seen).toEqual([{ text: "", doing: "Working on T-7" }]);
  });

  it("sends a file address's token as it is", async () => {
    const storage = memoryStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "i" });
    const { server, clock, auth, http } = fakeApi({ dialect: words, storage });
    server.on("POST", "/api/auth/refresh", () => json(200, { ...tokens(1, clock.now), access: "K-1" }));
    await auth.accessToken();
    expect(new URL(http.fileUrl("/uploads/x.png")).searchParams.get("token")).toBe("K-1");
  });

  it("changes nothing without one", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/tasks", () => json(200, { tasks: [{ id: 4, key: "K-4", owner: "person" }] }));
    expect(await api.tasks.list({ q: "K-4" })).toEqual([{ id: 4, key: "K-4", owner: "person" }]);
    expect(server.to("/api/tasks")[0]!.url.searchParams.get("q")).toBe("K-4");
  });
});

async function until(check: () => boolean): Promise<void> {
  for (let i = 0; i < 200 && !check(); i++) await new Promise((resolve) => setTimeout(resolve, 5));
  expect(check()).toBe(true);
}
