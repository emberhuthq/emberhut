import { describe, expect, it } from "vitest";
import { linkAction, originOf } from "./offers";

const link = { server: "https://other.example", code: "ABCD-EFGH" };

describe("pairing offers", () => {
  it("pairs by itself only on the paired server", () => {
    expect(linkAction({ ...link, server: "https://z.example" }, { server: "https://z.example", token: "t" }, [])).toBe("pair");
  });

  it("asks again on a server it was unpaired from", () => {
    expect(linkAction({ ...link, server: "https://z.example" }, { server: "https://z.example", token: null }, [])).toBe("offer-page");
  });

  it("offers another server in the popup alone once paired, on the consent page before", () => {
    expect(linkAction(link, { server: "https://z.example", token: "t" }, [])).toBe("offer-popup");
    expect(linkAction(link, { server: null, token: null }, [])).toBe("offer-page");
  });

  it("offers an origin once per session, whatever path its links name", () => {
    expect(linkAction(link, { server: null, token: null }, ["https://other.example"])).toBe("ignore");
    const pathed = { server: "https://other.example/a/b", code: "WXYZ-2345" };
    expect(linkAction(pathed, { server: null, token: null }, [originOf(link.server)])).toBe("ignore");
    expect(originOf("https://other.example:8443/emberhut")).toBe("https://other.example:8443");
  });
});
