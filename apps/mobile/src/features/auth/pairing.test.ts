import { describe, expect, it } from "vitest";
import { ed25519 } from "@noble/curves/ed25519.js";
import { createReach, createServerAddress, firstReachable, type Reach, type RouteAnswer } from "@/api";
import { decodeBase64Url, encodeBase64Url } from "@/connect/encoding";
import { notTheMachine, statement } from "@/connect/machineId";
import { createSessionStore } from "@/state/session";
import { fakeApi, fixedAddress, json, memoryStorage, tokens } from "@/test/fakes";
import { connectVectors } from "@/test/vectors";
import { formatCode, isComplete } from "./code";

describe("the pairing code field", () => {
  it("shows what you type as ABCD-EFGH, however you type or paste it", () => {
    expect(formatCode("abcd")).toBe("ABCD");
    expect(formatCode("abcde")).toBe("ABCD-E");
    expect(formatCode(" 4f7k 92qx ")).toBe("4F7K-92QX");
    expect(formatCode("4F7K-92QX")).toBe("4F7K-92QX");
    expect(formatCode("4F7K—92QX!!7")).toBe("4F7K-92QX");
    expect(isComplete("4F7K-92Q")).toBe(false);
    expect(isComplete("4F7K-92QX")).toBe(true);
  });
});

describe("pairing", () => {
  const device = { name: "Pixel 8", model: "Pixel 8" };

  function setup(answer: () => Response) {
    const { server, auth, api } = fakeApi();
    server.on("POST", "/api/auth/pair", answer);
    return { server, store: createSessionStore({ auth, api, address: fixedAddress(), device }) };
  }

  it("sends the name you gave this device, or its own when you left it empty", async () => {
    const { server, store } = setup(() => json(200, tokens(1, Date.parse("2026-11-24T10:00:00Z"))));
    await store.getState().start();
    expect(await store.getState().pair("4F7K-92QX", "  Android emulator (dev) ")).toBe(true);
    expect(server.to("/api/auth/pair")[0]?.body).toMatchObject({ code: "4F7K-92QX", name: "Android emulator (dev)", model: "Pixel 8" });
    await store.getState().signOut();
    await store.getState().pair("4F7K-92QX", " ");
    expect(server.to("/api/auth/pair")[1]?.body).toMatchObject({ name: "Pixel 8" });
  });

  it("shows the server's reason for a wrong, used or expired code, and a rate limit, without calling the phone refused", async () => {
    const { store } = setup(() => json(401, { ok: false, status: 401, error: "That code was already used. Make a new one." }));
    await store.getState().start();
    expect(await store.getState().pair("4F7K-92QX")).toBe(false);
    expect(store.getState()).toMatchObject({ pairError: "That code was already used. Make a new one.", refused: false });
  });

  it("knows the phone isn't on Emberhut's network when the first gate refuses it", async () => {
    const { store } = setup(() => json(401, { error: "not Emberhut's device" }));
    await store.getState().start();
    expect(await store.getState().pair("4F7K-92QX")).toBe(false);
    expect(store.getState().refused).toBe(true);
  });

  it("is let in again once a request gets past the gate", async () => {
    let gate = true;
    const { store } = setup(() =>
      gate
        ? json(401, { error: "not Emberhut's device" })
        : json(429, { ok: false, status: 429, error: "Too many wrong codes. Try again in 4 minutes." }),
    );
    await store.getState().start();
    await store.getState().pair("4F7K-92QX");
    gate = false;
    await store.getState().pair("4F7K-92QX");
    expect(store.getState()).toMatchObject({ refused: false, pairError: "Too many wrong codes. Try again in 4 minutes." });
  });
});

describe("the machine a pairing answer proves", () => {
  const device = { name: "Pixel 8", model: "Pixel 8" };
  const machine = { id: "3j6rajbbrd2yrrvdkoq2mouz5aqxwh5c", publicKey: "iJ6VrUsjsH5tpp2N75DjqJTSWs5mm3l979UsEGknM3g" };
  const other = { id: "qoasjogo65qh2cy62xmytganhyz74fla", publicKey: "2cHeyDHvhA9M_VrS73JVkCm-NoudiJz4GiVevuJ32qc" };
  const now = Date.parse("2026-11-24T10:00:00Z");

  function setup(reach?: Reach) {
    const { server, auth, api, storage } = fakeApi();
    let answer: unknown = { ...tokens(1, now), machine };
    server.on("POST", "/api/auth/pair", () => json(200, answer));
    const store = createSessionStore({ auth, api, address: fixedAddress(), device, reach });
    return { server, auth, storage, store, answerWith: (next: unknown) => void (answer = next) };
  }

  it("pins the key of the first machine a typed pairing reaches, and expects it from then on", async () => {
    const { auth, storage, store, answerWith } = setup();
    await store.getState().start();
    expect(await store.getState().pair("4F7K-92QX")).toBe(true);
    expect(store.getState().machineId).toBe(machine.id);
    expect(JSON.parse(storage.items.get("emberhut.machine") ?? "null")).toMatchObject(machine);

    await store.getState().signOut();
    answerWith({ ...tokens(2, now), machine: other });
    expect(await store.getState().pair("4F7K-92QX")).toBe(false);
    expect(store.getState().pairError).toBe(notTheMachine);
    expect(auth.signedIn).toBe(false);

    // Leaving `machine` out doesn't get past the pin either.
    answerWith(tokens(3, now));
    expect(await store.getState().pair("4F7K-92QX")).toBe(false);
    expect(auth.signedIn).toBe(false);
  });

  it("keeps the server it had when a pairing elsewhere is refused, for another machine or a wrong code", async () => {
    const { server, auth, api } = fakeApi();
    const address = createServerAddress(memoryStorage({ "emberhut.server": "https://home.example" }));
    let answer = () => json(200, { ...tokens(1, now), machine: other });
    server.on("POST", "/api/auth/pair", () => answer());
    const store = createSessionStore({ auth, api, address, device, reach: async () => "proved" });
    await store.getState().start();
    expect(store.getState().serverUrl).toBe("https://home.example");

    expect(await store.getState().pairLink({ id: machine.id, code: "ABCD-EFGH", routes: ["https://elsewhere.example"] })).toBe(false);
    expect(store.getState().pairError).toBe(notTheMachine);
    expect(store.getState().serverUrl).toBe("https://home.example");
    expect(address.url).toBe("https://home.example");

    answer = () => json(401, { ok: false, status: 401, error: "That code isn't right." });
    expect(await store.getState().pair("4F7K-92QX", undefined, "https://elsewhere.example")).toBe(false);
    expect(store.getState()).toMatchObject({ pairError: "That code isn't right.", serverUrl: "https://home.example" });
    expect(address.url).toBe("https://home.example");

    answer = () => json(200, { ...tokens(2, now), machine });
    expect(await store.getState().pair("4F7K-92QX", undefined, "https://elsewhere.example")).toBe(true);
    expect(store.getState().serverUrl).toBe("https://elsewhere.example");
    expect(address.url).toBe("https://elsewhere.example");
  });

  it("refuses an answer whose key doesn't give its id, and keeps none of its tokens", async () => {
    const { auth, storage, store, answerWith } = setup();
    answerWith({ ...tokens(1, now), machine: { id: other.id, publicKey: machine.publicKey } });
    await store.getState().start();
    expect(await store.getState().pair("4F7K-92QX")).toBe(false);
    expect(store.getState().pairError).toBe(notTheMachine);
    expect(auth.signedIn).toBe(false);
    expect(storage.items.has("emberhut.refresh")).toBe(false);
  });

  it("takes a server older than machine keys only on a first, typed pairing", async () => {
    const { store, storage, answerWith } = setup();
    answerWith(tokens(1, now));
    await store.getState().start();
    expect(await store.getState().pair("4F7K-92QX")).toBe(true);
    expect(store.getState().machineId).toBeNull();
    expect(storage.items.has("emberhut.machine")).toBe(false);
  });

  it("pairs a machine picked from the account only when the answer proves it is that one", async () => {
    const { store, answerWith } = setup();
    await store.getState().start();
    store.getState().pinMachine({ id: other.id, name: "Room 2" });
    expect(await store.getState().pair("4F7K-92QX")).toBe(false);
    expect(store.getState().pairError).toBe(notTheMachine);
    answerWith({ ...tokens(1, now), machine: other });
    expect(await store.getState().pair("4F7K-92QX")).toBe(true);
    expect(store.getState()).toMatchObject({ machineId: other.id, pin: null });
  });

  describe("from a link", () => {
    const link = (routes: string[], id = machine.id) => ({ id, code: "ABCD-EFGH", routes });

    it("pairs over the first route that proves the machine, in the link's order, and sends the link's code there", async () => {
      const asked: string[] = [];
      const reach: Reach = async (url, id) => {
        asked.push(url);
        expect(id).toBe(machine.id);
        return url === "http://192.168.1.10:18990" ? "none" : url === "http://10.0.0.5:18990" ? "proved" : "answered";
      };
      const { server, store } = setup(reach);
      await store.getState().start();
      // Plain http to a public address is never sent to (the typed address's rule), and a route is asked once.
      const routes = [
        "http://192.0.2.10:18990",
        "http://192.168.1.10:18990",
        "https://emberhut.example",
        "https://emberhut.example/",
        "http://10.0.0.5:18990",
      ];
      expect(await store.getState().pairLink(link(routes))).toBe(true);
      expect(asked).toEqual(["http://192.168.1.10:18990", "https://emberhut.example", "http://10.0.0.5:18990"]);
      expect(store.getState().serverUrl).toBe("http://10.0.0.5:18990");
      expect(server.to("/api/auth/pair")[0]?.body).toMatchObject({ code: "ABCD-EFGH" });
    });

    it("refuses a server that answers as another machine than the link's", async () => {
      const { store, auth } = setup(async () => "answered");
      await store.getState().start();
      expect(await store.getState().pairLink(link(["https://emberhut.example"], other.id))).toBe(false);
      expect(store.getState().pairError).toBe(notTheMachine);
      expect(auth.signedIn).toBe(false);
    });

    it("refuses a server that leaves its machine out: a link names an id", async () => {
      const { store, answerWith } = setup(async () => "answered");
      answerWith(tokens(1, now));
      await store.getState().start();
      expect(await store.getState().pairLink(link(["https://emberhut.example"]))).toBe(false);
      expect(store.getState().pairError).toBe(notTheMachine);
    });

    it("sends the code nowhere when no route answers, or none can be used from the phone", async () => {
      const { server, store } = setup(async () => "none");
      await store.getState().start();
      expect(await store.getState().pairLink(link(["https://emberhut.example"]))).toBe(false);
      expect(store.getState().pairError).toMatch(/Can't reach the machine/);
      const relayOnly = ["wss://relay.example/v1/sealed/" + machine.id, "http://emberhut.example:18990"];
      expect(await store.getState().pairLink(link(relayOnly))).toBe(false);
      expect(store.getState().pairError).toMatch(/None of the link's addresses/);
      expect(server.to("/api/auth/pair")).toEqual([]);
    });

    it("must be for the machine picked from the account", async () => {
      const { store } = setup(async () => "proved");
      await store.getState().start();
      store.getState().pinMachine({ id: other.id, name: "Room 2" });
      expect(await store.getState().pairLink(link(["https://emberhut.example"]))).toBe(false);
      expect(store.getState().pairError).toBe("That pairing link is for another machine, not Room 2.");
    });
  });
});

describe("probing a link's route before its code goes there", () => {
  const { keys } = connectVectors<{ keys: { name: string; seed: string; publicKey: string; machineId: string }[] }>("keys.json");
  const key = (name: string) => keys.find((k) => k.name === name)!;
  const machineKey = key("machine");
  const otherKey = key("other-machine");
  const fixedNonce = new Uint8Array(32).fill(7);
  const random = (size: number) => fixedNonce.slice(0, size);

  /** The machine's answer to a probe, signed as a server would sign it, with fields to change. */
  function answer(query: URLSearchParams, change: Partial<Record<"via" | "local" | "host" | "id", string>> = {}, signer = machineKey) {
    const fields = { id: signer.machineId, via: "direct", local: "192.168.1.10:18990", host: "192.168.1.10:18990", ...change };
    const nonce = decodeBase64Url(query.get("nonce") ?? "", 32);
    if (!nonce.ok) throw new Error("bad nonce");
    const text = new TextEncoder();
    const signed = statement("emberhut probe v1", [
      nonce.bytes,
      ...[fields.id, query.get("kind") ?? "", fields.via, fields.local, fields.host].map((f) => text.encode(f)),
    ]);
    const signature = ed25519.sign(signed, Buffer.from(signer.seed, "hex"));
    return { ...fields, publicKey: signer.publicKey, signature: encodeBase64Url(signature) };
  }

  function setup(probe: (query: URLSearchParams) => Response, status?: () => Response) {
    const { server } = fakeApi();
    server.on("GET", "/api/connect/probe", (request) => probe(request.url.searchParams));
    if (status) server.on("GET", "/api/auth/status", status);
    return { server, reach: createReach(server.fetch, random) };
  }

  it("counts a route proved when the machine signs this probe from the route's own address", async () => {
    const { server, reach } = setup((query) => json(200, answer(query)));
    expect(await reach("http://192.168.1.10:18990", machineKey.machineId)).toBe("proved");
    const sent = server.to("/api/connect/probe")[0]!.url.searchParams;
    expect(sent.get("kind")).toBe("lan");
    expect(sent.get("nonce")).toBe(encodeBase64Url(fixedNonce));
  });

  it("probes a name as a name route, bound to its host", async () => {
    const { reach } = setup((query) => json(200, answer(query, { local: "127.0.0.1:18990", host: "emberhut.example" })));
    expect(await reach("https://emberhut.example", machineKey.machineId)).toBe("proved");
    expect(await reach("https://emberhut.example:8443", machineKey.machineId)).toBe("none");
  });

  it("refuses another machine, a forged signature, an answer from elsewhere, and any other status", async () => {
    const cases: [string, (query: URLSearchParams) => Response][] = [
      ["another machine", (q) => json(200, answer(q, {}, otherKey))],
      [
        "a signature by another key",
        (q) => json(200, { ...answer(q), signature: answer(q, { id: otherKey.machineId }, otherKey).signature }),
      ],
      ["forwarded from another address", (q) => json(200, answer(q, { local: "192.168.1.11:18990", host: "192.168.1.11:18990" }))],
      ["relayed", (q) => json(200, answer(q, { via: "relayed", local: "" }))],
      ["the network gate's 401", () => json(401, { error: "not Emberhut's device" })],
      ["a server error", () => json(500, { error: "x" })],
      ["a page that isn't JSON", () => new Response("<html>login</html>", { status: 200 })],
    ];
    for (const [why, probe] of cases) {
      const { reach } = setup(probe, () => json(200, { mode: "report", bound: false, devices: [], signedIn: false }));
      expect(await reach("http://192.168.1.10:18990", machineKey.machineId), why).toBe("none");
    }
  });

  it("falls back, for a server older than the probe, to its sign-in status in Emberhut's exact shape only", async () => {
    const status = { mode: "report", bound: false, devices: [], signedIn: false };
    const older = (answer: () => Response) => setup(() => json(404, { error: "not found" }), answer).reach;
    expect(await older(() => json(200, status))("https://emberhut.example", machineKey.machineId)).toBe("answered");
    expect(await older(() => json(200, { ...status, mode: "open" }))("https://emberhut.example", machineKey.machineId)).toBe("none");
    expect(await older(() => json(200, { mode: "report" }))("https://emberhut.example", machineKey.machineId)).toBe("none");
    expect(await older(() => json(401, { error: "not Emberhut's device" }))("https://emberhut.example", machineKey.machineId)).toBe("none");
    expect(
      await setup(
        () => json(405, {}),
        () => json(200, status),
      ).reach("https://emberhut.example", machineKey.machineId),
    ).toBe("answered");
  });

  it("gives up on a route that doesn't answer in time, or can't be reached", async () => {
    const hangs = createReach(
      (_url, init) => new Promise((_resolve, reject) => init.signal?.addEventListener("abort", () => reject(new Error("aborted")))),
      random,
      10,
    );
    expect(await hangs("https://emberhut.example", machineKey.machineId)).toBe("none");
    const down = createReach(async () => Promise.reject(new TypeError("Network request failed")), random);
    expect(await down("https://emberhut.example", machineKey.machineId)).toBe("none");
  });

  it("takes the earliest route in order that proved the machine, then the earliest that answered, not the fastest", async () => {
    const slow = (ms: number, answer: RouteAnswer) => new Promise<RouteAnswer>((resolve) => setTimeout(() => resolve(answer), ms));
    const id = machineKey.machineId;
    expect(await firstReachable(["a", "b"], id, (url) => (url === "a" ? slow(30, "proved") : slow(1, "proved")))).toBe("a");
    expect(await firstReachable(["a", "b", "c"], id, async (url) => (url === "a" ? "answered" : url === "c" ? "proved" : "none"))).toBe(
      "c",
    );
    expect(await firstReachable(["a", "b"], id, async (url) => (url === "b" ? "answered" : "none"))).toBe("b");
    expect(await firstReachable(["a"], id, async () => "none")).toBeNull();
  });
});
