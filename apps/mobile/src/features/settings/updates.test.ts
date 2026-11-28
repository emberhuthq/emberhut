import { describe, expect, it } from "vitest";
import { storeLink, versionLine } from "./updates";

describe("where the phone's updates come from", () => {
  it("opens TestFlight on iOS, with its web page to fall back on", () => {
    expect(storeLink("ios")).toEqual({
      title: "Updates come through TestFlight",
      url: "itms-beta://",
      fallback: "https://testflight.apple.com/",
    });
  });

  it("opens the app's Play listing on Android", () => {
    const link = storeLink("android");
    expect(link.title).toBe("Updates come through Google Play");
    expect(link.url).toBe("market://details?id=com.emberhut.mobile");
    expect(link.fallback).toBe("https://play.google.com/store/apps/details?id=com.emberhut.mobile");
  });

  it("says the version and the build", () => {
    expect(versionLine("0.1.0", "6")).toBe("0.1.0 (build 6)");
    expect(versionLine("0.1.0", 6)).toBe("0.1.0 (build 6)");
    expect(versionLine("0.1.0", null)).toBe("0.1.0");
    expect(versionLine(undefined, undefined)).toBe("?");
  });
});
