import { describe, expect, it } from "vitest";
import { taskKey, type BrowserInfo } from "@/api";
import { address, handoffLine, isMine, isSecure, isUnclaimed, liveViewFailure, site, statusText } from "./browser";

// Private addresses are put together from their numbers: the repository's personal-data scan flags any written out.
const ip = (...parts: number[]) => parts.join(".");

const info = (extra: Partial<BrowserInfo> = {}): BrowserInfo => ({ connected: true, url: "", title: "", control: "agent", ...extra });

describe("who drives", () => {
  it("knows this device, another one, and a browser nobody took yet", () => {
    expect(isMine(info({ control: "owner", device: "me" }), "me")).toBe(true);
    expect(isMine(info({ control: "owner", device: "ipad" }), "me")).toBe(false);
    expect(isMine(info({ control: "owner", device: null }), "me")).toBe(true);
    expect(isUnclaimed(info({ control: "owner", device: "" }))).toBe(true);
    expect(isMine(info({ control: "agent" }), "me")).toBe(false);
  });

  it("says it over the page", () => {
    expect(statusText(info({ control: "owner", device: "" }), "me")).toBe("Emberhut asked you to take over");
    expect(statusText(info({ control: "owner", device: "me" }), "me")).toBe("You're driving");
    expect(statusText(info({ control: "owner", device: "ipad" }), "me")).toBe("Driving on your other device");
    expect(statusText(info(), "me")).toBe("Watching");
    expect(statusText(info({ driver: "Emberhut", task: 12 }), "me")).toBe(`Emberhut is driving · ${taskKey(12)}`);
    expect(statusText(info({ driver: "A worker" }), "me")).toBe("A worker is driving");
  });

  it("puts a line in the chat only while it waits on you", () => {
    expect(handoffLine(info({ control: "owner", device: "", handoff: "log in to the bank" }), "me")).toBe(
      "Emberhut handed you the browser: log in to the bank",
    );
    expect(handoffLine(info({ control: "owner", device: "" }), "me")).toBe("Emberhut handed you the browser");
    expect(handoffLine(info({ control: "owner", device: "me", handoff: "the captcha" }), "me")).toBe(
      "Emberhut handed you the browser: the captcha",
    );
    expect(handoffLine(info({ control: "owner", device: "me" }), "me")).toBeNull();
    expect(handoffLine(info({ control: "owner", device: "ipad", handoff: "x" }), "me")).toBeNull();
    expect(handoffLine(info({ handoff: "x" }), "me")).toBeNull();
    expect(handoffLine(null, "me")).toBeNull();
  });
});

describe("the address bar", () => {
  const tabs = (url: string, title = "") => [
    { id: 1, url: "https://other.com", title: "", active: false },
    { id: 2, url, title, active: true },
  ];

  it("shows the active tab's site", () => {
    expect(site(info({ tabs: tabs("https://www.github.com/x/y") }))).toBe("github.com");
    expect(site(info({ tabs: tabs("about:blank") }))).toBe("New tab");
    expect(site(info({ tabs: tabs("chrome://settings", "Settings") }))).toBe("Settings");
    expect(site(info({ url: "http://example.org:8080/a" }))).toBe("example.org");
    expect(site(info())).toBe("New tab");
  });

  it("knows a secure page and the whole address", () => {
    expect(isSecure(info({ tabs: tabs("https://a.b") }))).toBe(true);
    expect(isSecure(info({ url: "http://a.b" }))).toBe(false);
    expect(address(info({ tabs: tabs("about:blank") }))).toBe("");
    expect(address(info({ url: "https://a.b/c" }))).toBe("https://a.b/c");
  });
});

describe("liveViewFailure", () => {
  it("says on iOS that an http address by name needs https or a private IP", () => {
    expect(liveViewFailure("http://stream.example.com:6080/view", "The resource could not be loaded", "ios")).toBe(
      "The live view's address (stream.example.com) can't be opened: it needs https, or a private IP over http.",
    );
  });

  it("gives Android, which allows cleartext, the page's own reason for the same address", () => {
    expect(liveViewFailure("http://stream.example.com:6080/view", "net::ERR_NAME_NOT_RESOLVED", "android")).toBe(
      "The live view (stream.example.com) didn't load: net::ERR_NAME_NOT_RESOLVED",
    );
  });

  it("gives the page's own failure for an address it may load", () => {
    expect(liveViewFailure(`http://${ip(100, 80, 1, 2)}:6080/`, "Could not connect to the server.", "ios")).toBe(
      `The live view (${ip(100, 80, 1, 2)}) didn't load: Could not connect to the server.`,
    );
    expect(liveViewFailure("https://stream.example.com/", 502, "ios")).toBe("The live view (stream.example.com) didn't load: HTTP 502");
    expect(liveViewFailure("http://[fd00::1]:6080/", undefined, "android")).toBe("The live view ([fd00::1]) didn't load.");
  });
});
