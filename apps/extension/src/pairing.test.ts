import { describe, expect, it } from "vitest";
import { codeOf, linkOf, readPairing, serverOf } from "./pairing";

describe("pairing", () => {
  it("reads codes however they're typed", () => {
    expect(codeOf("abcd-efgh")).toBe("ABCD-EFGH");
    expect(codeOf(" ABCDEFGH ")).toBe("ABCD-EFGH");
    expect(codeOf("ABC")).toBeNull();
  });

  it("keeps a server's address without its final slash, http or https only", () => {
    expect(serverOf("https://emberhut.example.com/")).toBe("https://emberhut.example.com");
    expect(serverOf("http://emberhut.example:18990")).toBe("http://emberhut.example:18990");
    expect(serverOf("https://example.com/emberhut/")).toBe("https://example.com/emberhut");
    expect(serverOf("ftp://example.com")).toBeNull();
    expect(serverOf("https://user:pw@example.com")).toBeNull();
    expect(serverOf("nope")).toBeNull();
  });

  it("reads a pairing link, under a path too", () => {
    expect(linkOf("https://emberhut.example.com/chrome/pair#ABCD-EFGH")).toEqual({
      server: "https://emberhut.example.com",
      code: "ABCD-EFGH",
    });
    expect(linkOf("https://example.com/emberhut/chrome/pair#abcdefgh")).toEqual({
      server: "https://example.com/emberhut",
      code: "ABCD-EFGH",
    });
    expect(linkOf("https://emberhut.example.com/chrome/pair")).toBeNull();
    expect(linkOf("https://emberhut.example.com/other#ABCD-EFGH")).toBeNull();
    expect(linkOf("chrome://newtab")).toBeNull();
  });

  it("reads the popup's boxes: a link, or a code and an address", () => {
    expect(readPairing("https://z.example/chrome/pair#ABCD-EFGH", "")).toEqual({ server: "https://z.example", code: "ABCD-EFGH" });
    expect(readPairing("abcd-efgh", "https://z.example/")).toEqual({ server: "https://z.example", code: "ABCD-EFGH" });
    expect(readPairing("abcd-efgh", "")).toMatch(/address/);
    expect(readPairing("?", "https://z.example")).toMatch(/code/);
  });
});
