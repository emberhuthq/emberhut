import { describe, expect, it } from "vitest";
import type { Fetch } from "@/api";
import { json } from "@/test/fakes";
import { connectVectors } from "@/test/vectors";
import { createDirectory, DirectoryError } from "./directory";

interface Vectors {
  messages: { machines: { machines: unknown[] }; error: { error: string; code: string } };
  withUnknownFields: { machines: unknown };
}

const vectors = connectVectors<Vectors>("directory.json");
const baseUrl = "https://relay.example";

function setup(
  answer: (url: string, init: RequestInit) => Response | Promise<Response>,
  token: () => Promise<string | null> = async () => "session-token",
) {
  const sent: { url: string; init: RequestInit }[] = [];
  const fetch: Fetch = async (url, init) => {
    sent.push({ url, init });
    return answer(url, init);
  };
  return { sent, directory: createDirectory({ baseUrl, fetch, token }) };
}

const failure = async (work: Promise<unknown>): Promise<DirectoryError> => {
  const error = await work.then(
    () => null,
    (cause: unknown) => cause,
  );
  expect(error).toBeInstanceOf(DirectoryError);
  return error as DirectoryError;
};

describe("the account directory", () => {
  it("lists the account's machines with the directory's token, reading fields it doesn't know as the plain ones", async () => {
    const { sent, directory } = setup(() => json(200, vectors.withUnknownFields.machines));
    const machines = await directory.machines();
    expect(sent[0]?.url).toBe("https://relay.example/v1/machines");
    expect(new Headers(sent[0]?.init.headers).get("authorization")).toBe("Bearer session-token");
    expect(machines).toEqual(vectors.messages.machines.machines);
    expect(machines[1]).toMatchObject({ name: "Room 2", connected: false, lastSeen: null });
  });

  it("unlinks a machine by its id", async () => {
    const { sent, directory } = setup(() => new Response(null, { status: 204 }));
    await directory.unlink("3j6rajbbrd2yrrvdkoq2mouz5aqxwh5c");
    expect(sent[0]).toMatchObject({
      url: "https://relay.example/v1/machines/3j6rajbbrd2yrrvdkoq2mouz5aqxwh5c",
      init: { method: "DELETE" },
    });
  });

  it("calls the account signed out on the directory's sign-in code, and when there's no token to send", async () => {
    const { directory } = setup(() => json(401, vectors.messages.error));
    const error = await failure(directory.machines());
    expect(error).toMatchObject({ kind: "signIn", code: "sign-in", message: "Sign in to your account again." });

    const { sent, directory: signedOut } = setup(
      () => json(200, { machines: [] }),
      async () => null,
    );
    expect(await failure(signedOut.machines())).toMatchObject({ kind: "signIn" });
    expect(sent).toEqual([]);

    // A 401 without the directory's code (a proxy in front of it) is a refusal, not a reason to sign out.
    const proxy = setup(() => new Response("Unauthorized", { status: 401 }));
    expect(await failure(proxy.directory.machines())).toMatchObject({ kind: "refused" });
  });

  it("asks to try again later on 503 and 429, with Retry-After, and never signs out for them", async () => {
    const unavailable = setup(() => json(503, { error: "Try again soon.", code: "unavailable" }, { "Retry-After": "30" }));
    expect(await failure(unavailable.directory.machines())).toMatchObject({ kind: "later", code: "unavailable", retryAfter: 30 });
    const limited = setup(() => json(429, { error: "Too many requests.", code: "limit" }, { "Retry-After": "300" }));
    const error = await failure(limited.directory.unlink("x"));
    expect(error).toMatchObject({ kind: "later", code: "limit", retryAfter: 300, message: "The account is busy. Try again in 5 min." });
    const noHeader = setup(() => json(503, { error: "x", code: "unavailable" }));
    expect(await failure(noHeader.directory.machines())).toMatchObject({ kind: "later", retryAfter: undefined });
    // Clerk itself couldn't make a token: the same.
    const tokenless = setup(
      () => json(200, { machines: [] }),
      () => Promise.reject(new Error("offline")),
    );
    expect(await failure(tokenless.directory.machines())).toMatchObject({ kind: "later" });
  });

  it("shows any other refusal's sentence, and says when the directory can't be reached", async () => {
    const missing = setup(() => json(404, { error: "No such machine in this account.", code: "not-found" }));
    expect(await failure(missing.directory.unlink("x"))).toMatchObject({
      kind: "refused",
      code: "not-found",
      message: "No such machine in this account.",
    });
    const html = setup(() => new Response("<html>bad gateway</html>", { status: 502 }));
    expect(await failure(html.directory.machines())).toMatchObject({ kind: "refused", message: "The directory said no (HTTP 502)." });
    const down = setup(() => Promise.reject(new TypeError("Network request failed")));
    expect(await failure(down.directory.machines())).toMatchObject({ kind: "unreachable" });
    const garbled = setup(() => json(200, { nothing: true }));
    expect(await failure(garbled.directory.machines())).toMatchObject({ kind: "refused" });
  });

  it("skips a listed entry it can't show", async () => {
    const { directory } = setup(() => json(200, { machines: [{ id: "a" }, null, { id: "b", name: "B", publicKey: "k" }] }));
    expect(await directory.machines()).toEqual([
      { id: "b", name: "B", publicKey: "k", relayUrl: "", connected: false, lastSeen: null, linkedAt: 0 },
    ]);
  });
});
