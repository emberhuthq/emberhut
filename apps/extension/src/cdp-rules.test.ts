import { describe, expect, it } from "vitest";
import { aboutOwnWorld, callRefusal, forwards, keyRefusal, onElement, refusal, stripCookies, typesText, worldRefusal } from "./cdp-rules";

describe("the DevTools rules", () => {
  it("refuses everything the server's engine doesn't use, whatever reaches past the page first", () => {
    for (const method of [
      "Browser.close",
      "Storage.getCookies",
      "Storage.clearDataForOrigin",
      "Network.getAllCookies",
      "Network.loadNetworkResource",
      "Network.enable",
      "IO.read",
      "DOMStorage.getDOMStorageItems",
      "IndexedDB.requestData",
      "CacheStorage.requestEntries",
      "ServiceWorker.deliverPushMessage",
      "WebAuthn.enable",
      "WebAuthn.getCredentials",
      "Fetch.enable",
      "Page.setDownloadBehavior",
      "Target.createTarget",
      "Target.attachToTarget",
      "Target.setAutoAttach",
      "Extensions.loadUnpacked",
      "DOM.setAttributeValue",
      "DOM.setOuterHTML",
      "DOM.setNodeValue",
      "Input.dispatchDragEvent",
      "Input.imeSetComposition",
      "Runtime.compileScript",
    ]) {
      expect(refusal(method), method).toBe(`${method} isn't allowed in this Chrome`);
    }
  });

  it("lets the engine's own page commands through, on the tab's own session only", () => {
    for (const method of [
      "Page.navigate",
      "Runtime.evaluate",
      "Input.dispatchMouseEvent",
      "Accessibility.getFullAXTree",
      "Target.getTargetInfo",
    ]) {
      expect(refusal(method), method).toBeNull();
    }
    expect(refusal("Runtime.evaluate", "S1")).toMatch(/another target's session/);
  });

  it("knows typing: inserted text and keys that carry text, not bare keys", () => {
    expect(typesText("Input.insertText", { text: "hunter2" })).toBe(true);
    expect(typesText("Input.insertText", { text: "" })).toBe(false);
    expect(typesText("Input.dispatchKeyEvent", { type: "keyDown", key: "a", text: "a" })).toBe(true);
    expect(typesText("Input.dispatchKeyEvent", { type: "char", unmodifiedText: "a" })).toBe(true);
    expect(typesText("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "Tab" })).toBe(false);
    expect(typesText("Runtime.evaluate", { expression: "1" })).toBe(false);
  });

  it("lets Enter and Tab through, which type nothing; Space is typing", () => {
    expect(typesText("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", text: "\r", unmodifiedText: "\r" })).toBe(false);
    expect(typesText("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", text: "\n" })).toBe(false);
    expect(typesText("Input.dispatchKeyEvent", { type: "keyDown", key: "Tab", text: "\t" })).toBe(false);
    expect(typesText("Input.dispatchKeyEvent", { type: "keyDown", key: " ", text: " " })).toBe(true);
    expect(typesText("Input.dispatchKeyEvent", { type: "keyDown", text: "a\r" })).toBe(true);
  });

  it("refuses paste and editing commands on a key", () => {
    expect(keyRefusal("Input.dispatchKeyEvent", { key: "v", modifiers: 4 })).toMatch(/clipboard/);
    expect(keyRefusal("Input.dispatchKeyEvent", { code: "KeyV", modifiers: 2 })).toMatch(/clipboard/);
    expect(keyRefusal("Input.dispatchKeyEvent", { key: "Insert", modifiers: 8 })).toMatch(/clipboard/);
    expect(keyRefusal("Input.dispatchKeyEvent", { key: "a", commands: ["paste"] })).toMatch(/editing commands/);
    expect(keyRefusal("Input.dispatchKeyEvent", { key: "a", commands: ["selectAll"] })).toMatch(/editing commands/);
    expect(keyRefusal("Input.dispatchKeyEvent", { key: "v", text: "v", modifiers: 0 })).toBeNull();
    expect(keyRefusal("Input.dispatchKeyEvent", { key: "Enter", modifiers: 2 })).toBeNull();
  });

  it("knows a paste by its virtual key code too", () => {
    expect(keyRefusal("Input.dispatchKeyEvent", { windowsVirtualKeyCode: 86, modifiers: 2 })).toMatch(/clipboard/);
    expect(keyRefusal("Input.dispatchKeyEvent", { nativeVirtualKeyCode: 86, modifiers: 4 })).toMatch(/clipboard/);
    expect(keyRefusal("Input.dispatchKeyEvent", { windowsVirtualKeyCode: 45, modifiers: 8 })).toMatch(/clipboard/);
    expect(keyRefusal("Input.dispatchKeyEvent", { windowsVirtualKeyCode: 86, modifiers: 0 })).toBeNull();
    expect(keyRefusal("Input.dispatchKeyEvent", { windowsVirtualKeyCode: 45, modifiers: 0 })).toBeNull();
  });

  it("keeps the server out of Emberhut's own world, and its events out of what goes back", () => {
    const mine = new Set([7]);
    for (const [method, params] of [
      ["Runtime.evaluate", { expression: "1", contextId: 7 }],
      ["Runtime.evaluate", { expression: "1", contextId: 3 }],
      ["Runtime.evaluate", { expression: "1", uniqueContextId: "x" }],
      ["DOM.resolveNode", { backendNodeId: 1, executionContextId: 7 }],
      ["Page.addScriptToEvaluateOnNewDocument", { source: "", worldName: "emberhut-pill" }],
      ["Page.createIsolatedWorld", { frameId: "F", worldName: "emberhut-pill" }],
    ] as const) {
      expect(worldRefusal(method, params), method).toMatch(/another world/);
    }
    expect(worldRefusal("Runtime.evaluate", { expression: "1" })).toBeNull();
    expect(worldRefusal("Page.addScriptToEvaluateOnNewDocument", { source: "" })).toBeNull();
    expect(aboutOwnWorld("Runtime.executionContextCreated", { context: { id: 9, name: "emberhut-pill" } }, "emberhut-pill", mine)).toBe(
      true,
    );
    expect(aboutOwnWorld("Runtime.executionContextCreated", { context: { id: 3, name: "" } }, "emberhut-pill", mine)).toBe(false);
    expect(aboutOwnWorld("Runtime.executionContextDestroyed", { executionContextId: 7 }, "emberhut-pill", mine)).toBe(true);
    expect(aboutOwnWorld("Runtime.bindingCalled", { name: "__emberhutPause", executionContextId: 7 }, "emberhut-pill", mine)).toBe(true);
    expect(aboutOwnWorld("Page.loadEventFired", {}, "emberhut-pill", mine)).toBe(false);
  });

  it("calls functions only on an element, and checks each such element", () => {
    const fill = { objectId: "1.2.3", functionDeclaration: "function (t) {}", arguments: [{ value: "me@example.com" }] };
    expect(callRefusal("Runtime.callFunctionOn", fill)).toBeNull();
    expect(onElement("Runtime.callFunctionOn", fill)).toBe(true);
    expect(onElement("Runtime.callFunctionOn", { ...fill, arguments: [] })).toBe(true);
    expect(callRefusal("Runtime.callFunctionOn", { executionContextId: 1, functionDeclaration: "() => 1" })).toMatch(/no element/);
    expect(onElement("Runtime.evaluate", fill)).toBe(false);
  });

  it("sends back only the page's own events, never the network's", () => {
    expect(forwards("Page.javascriptDialogOpening")).toBe(true);
    expect(forwards("Inspector.detached")).toBe(true);
    expect(forwards("Network.requestWillBeSentExtraInfo")).toBe(false);
    expect(forwards("Network.responseReceivedExtraInfo")).toBe(false);
    expect(forwards("Page.loadEventFired", "S1")).toBe(false);
  });

  it("takes every cookie out of what goes back", () => {
    const event = {
      requestId: "1",
      associatedCookies: [{ cookie: { name: "sid", value: "secret" } }],
      headers: { Cookie: "sid=secret", "set-cookie": "sid=secret; HttpOnly", Accept: "text/html", Authorization: "Bearer x" },
      response: { headers: { "Set-Cookie": "a=b", "Content-Type": "text/html" }, cookies: [{ name: "a" }] },
    };
    expect(stripCookies(event)).toEqual({
      requestId: "1",
      headers: { Accept: "text/html" },
      response: { headers: { "Content-Type": "text/html" } },
    });
    expect(stripCookies("plain")).toBe("plain");
  });
});
