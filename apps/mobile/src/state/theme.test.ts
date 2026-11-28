import { describe, expect, it } from "vitest";
import type { StateStorage } from "zustand/middleware";
import type { ThemeChange, ThemeChoice, ThemePage } from "@/api";
import { createPrefsStore } from "@/state/prefs";
import { applyLocally, createThemeStore, currentTheme } from "@/state/theme";
import { fakeApi, json } from "@/test/fakes";
import { builtInTheme } from "@/theme/builtIn";

const paper = builtInTheme.presets.find((p) => p.id === "paper")!;
const purple = {
  id: "purple",
  name: "Purple",
  light: { ...paper.light, accent: "#7C3AED", accentSoft: "#EFE7FD" },
  dark: { ...paper.dark, accent: "#A78BFA", accentSoft: "#3A3352" },
};

const settle = () => new Promise((done) => setTimeout(done, 10));

function memory(saved = new Map<string, string>()): StateStorage & { saved: Map<string, string> } {
  return { saved, getItem: (k) => saved.get(k) ?? null, setItem: (k, v) => void saved.set(k, v), removeItem: (k) => void saved.delete(k) };
}

/** A stand-in for Emberhut's theme routes: one choice, resolved from the two presets; `down` makes it unreachable. */
function setup(choice: Partial<ThemeChoice> = {}) {
  const { server, api } = fakeApi();
  const net = { down: false, refuse: null as string | null, at: 0 };
  let saved: ThemeChoice = { ...builtInTheme.choice, ...choice };
  const page = (c: ThemeChoice): ThemePage => {
    const preset = c.preset === "purple" ? purple : paper;
    const tokens = { light: { ...preset.light }, dark: { ...preset.dark } };
    if (c.accent) tokens.light.accent = tokens.dark.accent = c.accent;
    return { ...builtInTheme, choice: c, tokens, presets: [paper, purple] };
  };
  server.on("GET", "/api/theme", () => {
    if (net.down) throw new TypeError("Network request failed");
    return json(200, page(saved));
  });
  server.on("POST", "/api/theme", ({ body }) => {
    if (net.down) throw new TypeError("Network request failed");
    if (net.refuse) return json(400, { ok: false, error: net.refuse });
    const { preview, ...change } = body as ThemeChange & { preview?: boolean };
    const next = { ...saved, ...change, at: preview ? saved.at : `2026-11-25T10:00:0${++net.at}Z` };
    if (!preview) saved = next;
    return json(200, { ok: true, ...page(next) });
  });
  const prefs = createPrefsStore(memory());
  const store = createThemeStore(api, prefs);
  return {
    server,
    net,
    prefs,
    store,
    posts: () =>
      server
        .to("/api/theme")
        .filter((r) => r.method === "POST")
        .map((r) => r.body),
  };
}

describe("theme sync", () => {
  it("shows the built-in paper in the phone's old appearance before Emberhut ever answers", () => {
    const { prefs } = setup();
    prefs.getState().setTheme("dark");
    expect(currentTheme(prefs.getState())).toMatchObject({ choice: { preset: "paper", appearance: "dark" }, tokens: builtInTheme.tokens });
  });

  it("caches the first answer and sends nothing when this phone was on System", async () => {
    const { store, prefs, posts } = setup();
    await store.getState().refresh();
    expect(prefs.getState().themeCache?.presets.map((p) => p.id)).toEqual(["paper", "purple"]);
    expect(prefs.getState().themeMigrated).toBe(true);
    expect(posts()).toEqual([]);
  });

  it("sends the old local appearance once, only when no device ever chose", async () => {
    const fresh = setup();
    fresh.prefs.getState().setTheme("dark");
    await fresh.store.getState().refresh();
    expect(fresh.posts()).toEqual([{ appearance: "dark" }]);
    expect(fresh.prefs.getState().themeCache?.choice).toMatchObject({ appearance: "dark", at: "2026-11-25T10:00:01Z" });
    await fresh.store.getState().refresh();
    expect(fresh.posts()).toHaveLength(1);

    const chosen = setup({ appearance: "light", at: "2026-11-20T08:00:00Z" });
    chosen.prefs.getState().setTheme("dark");
    await chosen.store.getState().refresh();
    expect(chosen.posts()).toEqual([]);
    expect(chosen.prefs.getState().themeCache?.choice.appearance).toBe("light");
  });

  it("fetches again only when the state says another device changed it", async () => {
    const { store, server } = setup({ at: "2026-11-20T08:00:00Z" });
    await store.getState().refresh();
    const gets = () => server.to("/api/theme").filter((r) => r.method === "GET").length;
    store.getState().noteState({ ...builtInTheme.choice, at: "2026-11-20T08:00:00Z" });
    await settle();
    expect(gets()).toBe(1);
    store.getState().noteState({ ...builtInTheme.choice, preset: "purple", at: "2026-11-25T09:00:00Z" });
    await settle();
    expect(gets()).toBe(2);
    store.getState().noteState(undefined); // a server on older code
    expect(gets()).toBe(2);
  });

  it("applies a preset at once and keeps the server's answer", async () => {
    const { store, prefs, posts } = setup();
    await store.getState().refresh();
    const sent = store.getState().choose({ preset: "purple", accent: null });
    expect(prefs.getState().themeCache?.tokens.light.accent).toBe("#7C3AED");
    await sent;
    expect(posts()).toEqual([{ preset: "purple", accent: null }]);
    expect(prefs.getState().themeCache?.choice).toMatchObject({ preset: "purple", at: "2026-11-25T10:00:01Z" });
  });

  it("switches a preset offline from the cache and sends it when Emberhut answers again", async () => {
    const { store, prefs, net, posts } = setup();
    await store.getState().refresh();
    net.down = true;
    await store.getState().choose({ preset: "purple", accent: null });
    await store.getState().choose({ appearance: "dark" });
    expect(currentTheme(prefs.getState())).toMatchObject({ choice: { preset: "purple", appearance: "dark" } });
    expect(prefs.getState().themeCache?.tokens.dark.accent).toBe("#A78BFA");
    expect(prefs.getState().themePending).toEqual({ preset: "purple", accent: null, appearance: "dark" });
    expect(store.getState().error).toBeNull();

    net.down = false;
    store.getState().noteState(builtInTheme.choice);
    await settle();
    expect(posts().at(-1)).toEqual({ preset: "purple", accent: null, appearance: "dark" });
    expect(prefs.getState().themePending).toBeNull();
    expect(prefs.getState().themeCache?.choice).toMatchObject({ preset: "purple", appearance: "dark" });
    expect(prefs.getState().themeCache?.choice.at).not.toBeNull();
  });

  it("tints the ember at once, offline too, keeping the preset, and sends the tint alone", async () => {
    const { store, prefs, net, posts } = setup({ preset: "purple" });
    await store.getState().refresh();
    const sent = store.getState().choose({ emberTint: "rose" });
    expect(prefs.getState().themeCache?.choice).toMatchObject({ preset: "purple", emberTint: "rose" });
    await sent;
    expect(posts()).toEqual([{ emberTint: "rose" }]);
    expect(prefs.getState().themeCache?.choice).toMatchObject({ preset: "purple", emberTint: "rose" });

    net.down = true;
    await store.getState().choose({ emberTint: "sage" });
    expect(currentTheme(prefs.getState()).choice.emberTint).toBe("sage");
    expect(prefs.getState().themePending).toEqual({ emberTint: "sage" });
  });

  it("keeps an offline change across a restart", async () => {
    const storage = memory();
    const { server, api } = fakeApi();
    server.on("POST", "/api/theme", () => {
      throw new TypeError("Network request failed");
    });
    const prefs = createPrefsStore(storage);
    prefs.getState().setThemeCache({ ...builtInTheme, presets: [paper, purple] });
    await createThemeStore(api, prefs).getState().choose({ preset: "purple", accent: null });
    const again = createPrefsStore(storage);
    await again.persist.rehydrate();
    expect(again.getState().themePending).toEqual({ preset: "purple", accent: null });
    expect(again.getState().themeCache?.choice.preset).toBe("purple");
  });

  it("goes back when the server refuses, and says why", async () => {
    const { store, prefs, net } = setup();
    await store.getState().refresh();
    net.refuse = "preset: one of paper, graphite";
    await store.getState().choose({ preset: "purple", accent: null });
    expect(prefs.getState().themeCache?.choice.preset).toBe("paper");
    expect(prefs.getState().themeCache?.tokens.light.accent).toBe("#B83D0C");
    expect(store.getState().error).toBe("preset: one of paper, graphite");
    expect(prefs.getState().themePending).toBeNull();
  });

  it("needs the server for a custom colour: previewed, applied, or refused offline", async () => {
    const { store, prefs, net, posts } = setup();
    await store.getState().refresh();
    const preview = await store.getState().preview("#BE123C");
    expect(preview.tokens.light.accent).toBe("#BE123C");
    expect(prefs.getState().themeCache?.choice.accent).toBeNull(); // nothing saved, nothing applied

    net.down = true;
    await store.getState().choose({ accent: "#BE123C" }, preview);
    expect(prefs.getState().themeCache?.choice.accent).toBeNull();
    expect(prefs.getState().themePending).toBeNull();
    expect(store.getState().error).toMatch(/needs Emberhut's server/);

    net.down = false;
    await store.getState().choose({ accent: "#BE123C" }, preview);
    expect(posts().at(-1)).toEqual({ accent: "#BE123C" });
    expect(prefs.getState().themeCache?.choice.accent).toBe("#BE123C");
    expect(store.getState().error).toBeNull();
  });
});

describe("a change before the server answers", () => {
  const page = { ...builtInTheme, presets: [paper, purple] };

  it("resolves a preset from the cached ones, and can't resolve one it doesn't have", () => {
    expect(applyLocally(page, { preset: "purple", accent: null })?.tokens.dark.accent).toBe("#A78BFA");
    expect(applyLocally(page, { preset: "teal", accent: null })).toBeNull();
  });

  it("clears a custom accent back to its preset's", () => {
    const custom = {
      ...page,
      choice: { ...page.choice, accent: "#BE123C" },
      tokens: { light: { ...paper.light, accent: "#BE123C" }, dark: paper.dark },
    };
    expect(applyLocally(custom, { accent: null })).toMatchObject({
      choice: { preset: "paper", accent: null },
      tokens: { light: { accent: "#B83D0C" } },
    });
  });

  it("knows a custom accent only from its preview", () => {
    expect(applyLocally(page, { accent: "#BE123C" })).toBeNull();
  });
});
