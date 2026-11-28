import { describe, expect, it } from "vitest";
import { firstMessage } from "./first-message";

const about = { version: "0.1.0", os: "mac", agent: "Chrome/131.0" };

describe("the first message", () => {
  it("pairs with the code alone: no account, no system, no token for another server", () => {
    const message = firstMessage(
      { server: "https://new.example", code: "ABCD-EFGH" },
      { server: "https://old.example", token: "t" },
      about,
    );
    expect(message).toEqual({ method: "pair", params: { protocol: 1, code: "ABCD-EFGH" } });
  });

  it("hands its old token only to the server that gave it", () => {
    const message = firstMessage({ server: "https://z.example", code: "ABCD-EFGH" }, { server: "https://z.example", token: "t" }, about);
    expect(message.params).toEqual({ protocol: 1, code: "ABCD-EFGH", old: "t" });
  });

  it("says what it is in the hello, to the paired server", () => {
    const message = firstMessage(null, { server: "https://z.example", token: "t" }, about);
    expect(message).toEqual({ method: "hello", params: { protocol: 1, token: "t", ...about } });
  });
});
