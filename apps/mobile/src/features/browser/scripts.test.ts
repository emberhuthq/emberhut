import { describe, expect, it, vi } from "vitest";
import { injected, keyScript, pasteScript, screenSize, typedChange } from "./scripts";

describe("live view scripts", () => {
  it("report the stream to the app every 2 s: its pictures so far, and whether its connection is open", () => {
    vi.useFakeTimers();
    const posted: unknown[] = [];
    const page: Record<string, unknown> = {
      ReactNativeWebView: { postMessage: (m: string) => posted.push(JSON.parse(m)) },
      videoChunksReceived: 0,
      selkiesTransport: null,
    };
    // The page's window, as the WebView runs the script in it (its own scripts publish the stream's state there).
    new Function("window", "document", injected("view", null))(page, {
      createElement: () => ({}),
      documentElement: { appendChild: () => undefined },
      addEventListener: () => undefined,
    });
    page.selkiesTransport = { readyState: 1 };
    vi.advanceTimersByTime(2000);
    page.videoChunksReceived = 42;
    vi.advanceTimersByTime(2000);
    page.selkiesTransport = { readyState: 3 };
    vi.advanceTimersByTime(2000);
    vi.useRealTimers();
    // Both modes carry it: the view and the driver.
    expect(injected("control", null)).toContain('"emberhut-stream"');
    expect(posted).toEqual([
      { type: "emberhut-stream", chunks: 0, open: false },
      { type: "emberhut-stream", chunks: 0, open: true },
      { type: "emberhut-stream", chunks: 42, open: true },
      { type: "emberhut-stream", chunks: 42, open: false },
    ]);
  });

  it("gives the view the screen's size, and the driver the keys and touch", () => {
    const view = injected("view", { width: 1920, height: 1080 });
    expect(view).toContain("width: 1920, height: 1080");
    expect(view).not.toContain("__emberhutKey");
    const control = injected("control", { width: 1920, height: 1080 });
    expect(control).toContain("window.__emberhutKey");
    expect(control).toContain("touchstart");
    expect(control).not.toContain("stream_resolution");
    expect(screenSize(800, 600)).toContain("width: 800, height: 600");
    // Each must parse as a script, as the WebView runs it.
    expect(() => new Function(view)).not.toThrow();
    expect(() => new Function(control)).not.toThrow();
  });

  it("splices in only a size of two positive numbers", () => {
    expect(screenSize(1919.6, 1080)).toContain("width: 1920, height: 1080");
    const hostile = "1); alert(1); (1" as unknown as number;
    expect(screenSize(hostile, 1080)).toBe("");
    expect(screenSize("800" as unknown as number, 600)).toContain("width: 800, height: 600");
    expect(screenSize(0, 600)).toBe("");
    expect(screenSize(Number.NaN, 600)).toBe("");
    expect(screenSize(800, Number.POSITIVE_INFINITY)).toBe("");
    expect(injected("view", { width: hostile, height: 1080 })).not.toContain("alert");
  });

  it("quotes what goes into the page", () => {
    expect(keyScript("Enter", "Enter", 13)).toBe(
      'window.__emberhutKey && __emberhutKey("Enter", "Enter", 13, {"ctrl":false,"shift":false}); true;',
    );
    expect(pasteScript('a"b')).toBe('window.__emberhutPaste && __emberhutPaste("a\\"b"); true;');
  });

  it("turns the typing row's edits into keys", () => {
    expect(typedChange("hel", "hello")).toEqual({ backspaces: 0, text: "lo" });
    expect(typedChange("hello", "he")).toEqual({ backspaces: 3, text: "" });
    expect(typedChange("cat", "dog")).toEqual({ backspaces: 3, text: "dog" });
  });
});
