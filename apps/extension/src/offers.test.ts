import { describe, expect, it } from "vitest";
import { codeOfOffer, linkAction, offerId, originOf } from "./offers";

const link = { server: "https://other.example", code: "ABCD-EFGH" };

describe("pairing offers", () => {
  it("pairs by itself only on the paired server", () => {
    expect(linkAction({ ...link, server: "https://z.example" }, { server: "https://z.example", token: "t" }, [])).toBe("pair");
  });

  it("offers another server in the popup alone once paired, with a notification before", () => {
    expect(linkAction(link, { server: "https://z.example", token: "t" }, [])).toBe("offer-popup");
    expect(linkAction(link, { server: null, token: null }, [])).toBe("offer-notification");
  });

  it("offers an origin once per session, whatever path its links name", () => {
    expect(linkAction(link, { server: null, token: null }, ["https://other.example"])).toBe("ignore");
    const pathed = { server: "https://other.example/a/b", code: "WXYZ-2345" };
    expect(linkAction(pathed, { server: null, token: null }, [originOf(link.server)])).toBe("ignore");
    expect(originOf("https://other.example:8443/emberhut")).toBe("https://other.example:8443");
  });

  it("names the code in the notification, so a click answers that offer only", () => {
    expect(codeOfOffer(offerId(link))).toBe("ABCD-EFGH");
    expect(codeOfOffer("handoff")).toBeNull();
  });
});
