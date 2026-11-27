import { describe, expect, it } from "vitest";
import { LINK_SCHEME } from "@/api";
import { isReport, isReportPage, reportTapScript, reportTitle, sandboxed, stem, tappedLink, viewerKind } from "./files";

describe("viewerKind", () => {
  it("goes by the extension, any case", () => {
    expect(viewerKind("IMG_0001.HEIC")).toBe("image");
    expect(viewerKind("clip.mov")).toBe("video");
    expect(viewerKind("review-pack.pdf")).toBe("pdf");
    expect(viewerKind("example-report.html")).toBe("html");
    expect(viewerKind("notes.md")).toBe("text");
    expect(viewerKind("archive.zip")).toBe("doc");
    expect(viewerKind("README")).toBe("doc");
    expect(isReport("a.htm")).toBe(true);
    expect(stem("launch-postmortem.html")).toBe("launch-postmortem");
  });
});

describe("reportTitle", () => {
  it("reads the title, else the first heading, as plain text", () => {
    expect(reportTitle("<html><head><title>Q3 &amp; Q4\n plan</title></head></html>")).toBe("Q3 & Q4 plan");
    expect(reportTitle("<body><h1 class=x>Why <b>downloads</b> fail</h1></body>")).toBe("Why downloads fail");
    expect(reportTitle("<body><p>no title</p></body>")).toBeNull();
    expect(reportTitle("<title> </title>")).toBeNull();
  });
});

describe("sandboxed", () => {
  it("puts the policy first, after the doctype, and the tap script right after it", () => {
    const page = sandboxed("<!DOCTYPE html><html><script>fetch('/x')</script></html>", "t0k");
    expect(page.startsWith('<!DOCTYPE html><meta http-equiv="Content-Security-Policy"')).toBe(true);
    expect(page).toContain("connect-src 'none'");
    expect(page.indexOf('const token = "t0k"')).toBeLessThan(page.indexOf("fetch('/x')"));
    expect(sandboxed("<p>hi</p>", "t0k").startsWith("<meta http-equiv")).toBe(true);
  });

  it("refuses a token that could break out of its script", () => {
    expect(() => sandboxed("<p>hi</p>", '"</script><script>x')).toThrow();
  });
});

describe("the report viewer", () => {
  it("loads only its own page", () => {
    expect(isReportPage("about:blank")).toBe(true);
    expect(isReportPage("about:blank#results")).toBe(true);
    for (const url of [
      "data:text/html,<script>x</script>",
      "https://evil.example",
      "intent://x#Intent;end",
      `${LINK_SCHEME}task/1`,
      "about:srcdoc",
      "file:///etc/hosts",
    ]) {
      expect(isReportPage(url), url).toBe(false);
    }
  });

  it("opens only http(s) links that were tapped, sent by the viewer's own script", () => {
    const tap = (open: string, token = "t0k") => JSON.stringify({ token, open });
    expect(tappedLink(tap("https://github.com/a"), "t0k")).toBe("https://github.com/a");
    expect(tappedLink(tap("intent://x"), "t0k")).toBeNull();
    expect(tappedLink(tap("javascript:alert(1)"), "t0k")).toBeNull();
    expect(tappedLink("not json", "t0k")).toBeNull();
    // A page's own scripts don't know the token.
    expect(tappedLink(JSON.stringify({ open: "https://evil.example/" }), "t0k")).toBeNull();
    expect(tappedLink(tap("https://evil.example/", "guess"), "t0k")).toBeNull();
    expect(tappedLink(tap("https://evil.example/", ""), "")).toBeNull();
  });

  /** The tap script run against a stand-in page; returns the click handler and what reached the real bridge. */
  function runTapScript() {
    class FakeEvent {
      constructor(
        readonly at: object,
        readonly isTrusted: boolean,
      ) {}
      get target() {
        return this.at;
      }
      preventDefault() {}
    }
    class FakeElement {
      constructor(readonly link: string) {}
      getAttribute() {
        return this.link;
      }
      closest() {
        return this;
      }
    }
    class FakeAnchor extends FakeElement {
      get href() {
        return this.link;
      }
    }
    const sent: string[] = [];
    const bridge = { postMessage: (m: string) => void sent.push(m) };
    let onClick: (e: object) => void = () => undefined;
    const document = { currentScript: null, addEventListener: (_: string, f: (e: object) => void) => (onClick = f) };
    const env = { ReactNativeWebView: bridge };
    new Function("window", "document", "Event", "Element", "HTMLAnchorElement", reportTapScript("t0k"))(
      env,
      document,
      FakeEvent,
      FakeElement,
      FakeAnchor,
    );
    const click = (link: string, isTrusted = true) => onClick(new FakeEvent(new FakeAnchor(link), isTrusted));
    return { sent, bridge, click, FakeEvent, FakeAnchor };
  }

  it("posts a real tap through the bridge it found, even after the page replaces it", () => {
    const { sent, bridge, click } = runTapScript();
    const stolen: string[] = [];
    bridge.postMessage = (m: string) => void stolen.push(m);
    click("https://github.com/a");
    click("https://evil.example/", false);
    click("#section");
    expect(stolen).toEqual([]);
    expect(sent.map((m) => tappedLink(m, "t0k"))).toEqual(["https://github.com/a"]);
  });

  it("keeps the token from a page that plants toJSON or replaces the getters after it ran", () => {
    const { sent, click, FakeEvent, FakeAnchor } = runTapScript();
    const leaked: unknown[] = [];
    const proto = Object.prototype as { toJSON?: () => unknown };
    proto.toJSON = function (this: unknown) {
      leaked.push(this);
      return { token: "t0k", open: "https://evil.example/" };
    };
    Object.defineProperty(FakeEvent.prototype, "target", { get: () => new FakeAnchor("https://evil.example/") });
    Object.defineProperty(FakeAnchor.prototype, "href", { get: () => "https://evil.example/" });
    try {
      click("https://github.com/a");
    } finally {
      delete proto.toJSON;
    }
    expect(leaked).toEqual([]);
    expect(sent.map((m) => tappedLink(m, "t0k"))).toEqual(["https://github.com/a"]);
  });

  it("gives a page no way to its listener through a getter's caller, so a saved tap can't be replayed", () => {
    const { sent, click } = runTapScript();
    const reached: unknown[] = [];
    // A non-strict getter, as a page would plant: its caller is the function that read it, unless that one is strict.
    const getter = new Function("reached", "return function () { reached.push(arguments.callee.caller); };")(reached) as () => void;
    // eslint-disable-next-line no-extend-native -- the test plants what a hostile page would, and removes it after.
    Object.defineProperty(String.prototype, "0", { get: getter, configurable: true });
    try {
      click("", false);
      click("#top", false);
    } finally {
      delete (String.prototype as unknown as Record<string, unknown>)["0"];
    }
    expect(reached.filter((f) => typeof f === "function")).toEqual([]);
    expect(sent).toEqual([]);
  });
});
