import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { createDialect, identityDialect, parseDialect } from "./dialect";
import { readMessage, writeMessage } from "./messages";
import { BY_ASSISTANT, INITIALIZED, Msg } from "./protocol";

// The cases the desktop client and the mobile app answer too.
interface Cases {
  dialect: unknown;
  texts: Record<"read" | "write", [string, string][]>;
  documents: Record<"read" | "write", [unknown, unknown][]>;
}
const cases = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../../crates/emberhut-client/tests/fixtures/dialect.json", import.meta.url)), "utf8"),
) as Cases;

// A made-up server dialect: it names its messages "other.*", calls the assistant "helper" and starts task keys with "K-".
const other = createDialect(
  parseDialect(
    JSON.stringify({
      words: [["helper", "assistant"]],
      prefixes: [
        ["other.", "emberhut."],
        ["K-", "T-"],
      ],
    }),
  ),
);

describe("parseDialect", () => {
  it("is empty, and rewrites nothing, when unset or blank", () => {
    for (const unset of [undefined, "", "  \n", "{}"]) expect(createDialect(parseDialect(unset))).toBe(identityDialect);
    const message = { method: Msg.start, params: { by: "helper" } };
    expect(readMessage(identityDialect, message)).toBe(message);
    expect(writeMessage(identityDialect, message)).toBe(message);
  });

  it("refuses a malformed dialect, naming what's wrong", () => {
    expect(() => parseDialect("{")).toThrow("isn't JSON");
    expect(() => parseDialect("[]")).toThrow("isn't a JSON object");
    expect(() => parseDialect('{"names":[]}')).toThrow("unknown section: names");
    expect(() => parseDialect('{"words":[["a"]]}')).toThrow("words isn't a list");
    expect(() => parseDialect('{"prefixes":[["a","a"]]}')).toThrow("prefixes isn't a list");
  });
});

describe("the shared cases", () => {
  const shared = createDialect(parseDialect(JSON.stringify(cases.dialect)));
  it.each(cases.texts.read)("read %j as %j", (given, expected) => expect(shared.fromServer(given)).toBe(expected));
  it.each(cases.texts.write)("write %j as %j", (given, expected) => expect(shared.toServer(given)).toBe(expected));
  it.each(cases.documents.read)("read the document %j", (given, expected) => expect(shared.jsonFromServer(given)).toEqual(expected));
  it.each(cases.documents.write)("write the document %j", (given, expected) => expect(shared.jsonToServer(given)).toEqual(expected));
});

describe("the socket's messages, built with a dialect", () => {
  it("reads a named message's name and params in the extension's words", () => {
    expect(readMessage(other, { method: "other.start" })).toEqual({ method: Msg.start });
    expect(readMessage(other, { method: "other.handback", params: { by: "helper", reason: "K-4 is done" } })).toEqual({
      method: Msg.handback,
      params: { by: BY_ASSISTANT, reason: "T-4 is done" },
    });
  });

  it("writes them back in the server's", () => {
    expect(writeMessage(other, { method: Msg.blocked, params: { tabId: 3, host: "example.test" } })).toEqual({
      method: "other.blocked",
      params: { tabId: 3, host: "example.test" },
    });
    expect(writeMessage(other, { method: Msg.stopped, params: { why: "a break from T-2", until: 5, turn: false } })).toEqual({
      method: "other.stopped",
      params: { why: "a break from K-2", until: 5, turn: false },
    });
    expect(writeMessage(other, { method: INITIALIZED, params: {} })).toEqual({ method: INITIALIZED, params: {} });
  });

  it("keeps a token as it is", () => {
    expect(readMessage(other, { method: "other.paired", params: { token: "K-1abc", name: "helper" } })).toEqual({
      method: Msg.paired,
      params: { token: "K-1abc", name: "assistant" },
    });
  });

  it("leaves the pages' own text alone: relayed calls, their answers, the browser's events and the first message's params", () => {
    const call = { id: 7, method: "chrome.debugger.sendCommand", params: [{ tabId: 1 }, "Input.insertText", { text: "helper K-1" }] };
    expect(readMessage(other, call)).toEqual(call);
    const answer = { id: 7, result: { value: "helper" } };
    expect(writeMessage(other, answer)).toBe(answer);
    const event = { method: "chrome.tabs.onCreated", params: [{ id: 1, title: "T-1 helper" }] };
    expect(writeMessage(other, event)).toEqual(event);
    const hello = { method: "hello", params: { protocol: 1, token: "T-1", version: "0.1.0" } };
    expect(writeMessage(other, hello)).toEqual(hello);
  });

  it("leaves a word that isn't a message's start alone", () => {
    expect(readMessage(other, { method: "another.start" })).toEqual({ method: "another.start" });
  });
});
