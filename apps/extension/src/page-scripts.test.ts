import { Script, runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";
import { CLOSING_BINDING, ELEMENT_CHECK, FOCUS_CHECK, GUARD, PILL, PILL_BINDING, SECRET_MEMORY, UNPILL, pointAt } from "./page-scripts";

/** Runs a page script's text against a stand-in document whose focus is `focus`, remembering `was` as secret. */
function focusOf(focus: unknown, was: WeakSet<object> | undefined = undefined): unknown {
  return runInNewContext(FOCUS_CHECK, { document: { activeElement: focus }, __emberhutWasSecret: was });
}

const input = (type: string, autocomplete = "") => ({ tagName: "INPUT", type, getAttribute: () => autocomplete });

/** SECRET_MEMORY run over a stand-in document holding `fields`: its memory, and its watcher's callback. */
function memoryOver(fields: object[]): { was: WeakSet<object>; changed: (records: object[]) => void } {
  let changed: (records: object[]) => void = () => undefined;
  class Watcher {
    constructor(callback: (records: object[]) => void) {
      changed = callback;
    }
    observe(): void {}
  }
  const root = { nodeType: 1, tagName: "HTML", getAttribute: () => null, querySelectorAll: () => fields };
  const sandbox: Record<string, unknown> = { document: { documentElement: root }, MutationObserver: Watcher, WeakSet };
  runInNewContext(SECRET_MEMORY, sandbox);
  return { was: sandbox.__emberhutWasSecret as WeakSet<object>, changed };
}

describe("the page scripts", () => {
  it("are scripts a page can run", () => {
    for (const script of [PILL, UNPILL, GUARD, FOCUS_CHECK, pointAt(1, 2, "mousePressed")]) {
      expect(() => new Script(script)).not.toThrow();
    }
    expect(() => new Script(`(${ELEMENT_CHECK})`)).not.toThrow();
    expect(PILL).toContain(`${PILL_BINDING}("pause")`);
    expect(PILL).toContain("Emberhut is driving");
    expect(PILL).toContain('mode: "closed"');
    expect(PILL).toContain('setAttribute("aria-hidden", "true")');
    expect(GUARD).toContain(CLOSING_BINDING);
    // The page's CSS can't hide the pill, and letting go stops what keeps it up.
    expect(PILL).toContain("display:block !important");
    expect(PILL).toContain("setInterval(draw, 1000)");
    expect(UNPILL).toContain("__emberhutStop?.()");
  });

  it("finds a password or code field with the focus", () => {
    expect(focusOf(input("password"))).toBe("secret");
    expect(focusOf(input("text", "one-time-code"))).toBe("secret");
    expect(focusOf(input("text", "username current-password"))).toBe("secret");
    expect(focusOf(input("email", "email"))).toBe("plain");
    expect(focusOf(null)).toBe("plain");
  });

  it("looks into open shadow roots and same-site frames, and can't into another site's", () => {
    const host = { tagName: "MY-FIELD", shadowRoot: { activeElement: input("password") } };
    expect(focusOf(host)).toBe("secret");
    const frame = { tagName: "IFRAME", contentDocument: { activeElement: input("password") } };
    expect(focusOf(frame)).toBe("secret");
    expect(focusOf({ tagName: "IFRAME", contentDocument: null })).toBe("unknown");
  });

  it("remembers a field that was a password field after its type changes", () => {
    const field = input("password");
    const plain = input("text");
    const { was, changed } = memoryOver([field, plain]);
    expect(was.has(field)).toBe(true);
    expect(was.has(plain)).toBe(false);
    field.type = "text";
    expect(focusOf(field)).toBe("plain");
    expect(focusOf(field, was)).toBe("secret");
    // A field turned from password to text after Emberhut came: its old type is remembered.
    const later = input("text");
    changed([{ type: "attributes", attributeName: "type", oldValue: "password", target: later }]);
    expect(focusOf(later, was)).toBe("secret");
    const check = runInNewContext(`(${ELEMENT_CHECK})`, { __emberhutWasSecret: was }) as (this: unknown) => boolean;
    expect(check.call(later)).toBe(true);
    // A password field added later is remembered too.
    const added = { ...input("password"), nodeType: 1, querySelectorAll: () => [] };
    changed([{ type: "childList", addedNodes: [added] }]);
    expect(was.has(added)).toBe(true);
  });

  it("checks the element a fill is called on", () => {
    const check = runInNewContext(`(${ELEMENT_CHECK})`) as (this: unknown) => boolean;
    expect(check.call(input("password"))).toBe(true);
    expect(check.call(input("text"))).toBe(false);
  });
});
