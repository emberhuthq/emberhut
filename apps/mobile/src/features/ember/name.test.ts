import { describe, expect, it } from "vitest";
import { ApiError, type EmberName, type EmberNameReply } from "@/api";
import { chatTitle, cleanEmberName, customEmberName, defaultEmberName, emberName, renameEmber, type RenameIo } from "./name";

const otto: EmberName = { name: "Pip", custom: true, at: "2026-11-28T10:00:00.000Z" };

describe("the ember's name in a state", () => {
  it("is yours when you gave one, else Your ember", () => {
    expect(emberName({ emberName: otto })).toBe("Pip");
    expect(customEmberName({ emberName: otto })).toBe("Pip");
    expect(emberName({ emberName: defaultEmberName })).toBe("Your ember");
    expect(customEmberName({ emberName: defaultEmberName })).toBeUndefined();
  });

  it("is Your ember from an older server", () => {
    expect(emberName({})).toBe("Your ember");
    expect(emberName({ emberName: null })).toBe("Your ember");
    expect(emberName(undefined)).toBe("Your ember");
  });
});

describe("a name as the server keeps it", () => {
  it("is trimmed", () => {
    expect(cleanEmberName("  Juniper ")).toEqual({ ok: true, name: "Juniper" });
    expect(cleanEmberName("Émile 🔥")).toEqual({ ok: true, name: "Émile 🔥" });
  });

  it("goes back to the default when blank or the default's words", () => {
    for (const blank of ["", "   ", "\n", "Your ember", " YOUR EMBER "]) expect(cleanEmberName(blank)).toEqual({ ok: true, name: null });
  });

  it("has 24 characters at most, counted as people see them", () => {
    expect(cleanEmberName("🔥".repeat(24))).toEqual({ ok: true, name: "🔥".repeat(24) });
    expect(cleanEmberName("x".repeat(25))).toEqual({ ok: false, error: "24 characters at most" });
  });

  it("is one line of plain text", () => {
    for (const bad of ["Ot\nto", "Ot\tto", "Ot\u0007to", "Ot‮to", "Ot​to"])
      expect(cleanEmberName(bad)).toEqual({ ok: false, error: "One line of plain text" });
  });
});

/** A rename's world: the name on screen, every name shown in turn, and what the server answers. */
function world(server: (name: string | null) => Promise<EmberNameReply>, start?: EmberName) {
  let current = start;
  const shown: string[] = [];
  const sent: (string | null)[] = [];
  const io: RenameIo = {
    current: () => current,
    show: (name) => {
      current = name;
      shown.push(name.name);
    },
    send: (name) => {
      sent.push(name);
      return server(name);
    },
  };
  return { io, shown, sent, now: () => current };
}

describe("renaming", () => {
  it("shows the new name at once, then the server's", async () => {
    const { io, shown, sent, now } = world(async (name) => ({ ok: true, name: name ?? "Your ember", custom: true, at: "later" }));
    const outcome = await renameEmber(" Juniper ", io);
    expect(outcome).toEqual({ ok: true, name: { name: "Juniper", custom: true, at: "later" } });
    expect(sent).toEqual(["Juniper"]);
    expect(shown).toEqual(["Juniper", "Juniper"]);
    expect(now()?.at).toBe("later");
  });

  it("goes back to the name before, with the server's reason, when it refuses", async () => {
    const { io, shown, now } = world(() => Promise.reject(new ApiError(400, "name: one line of plain text")), otto);
    const outcome = await renameEmber("Juniper", io);
    expect(outcome).toEqual({ ok: false, error: "One line of plain text" });
    expect(shown).toEqual(["Juniper", "Pip"]);
    expect(now()).toEqual(otto);
  });

  it("goes back when the server can't be reached", async () => {
    const { io, now } = world(() => Promise.reject(new ApiError(0, "timed out")), otto);
    expect(await renameEmber("Juniper", io)).toEqual({ ok: false, error: "Can't reach your ember" });
    expect(now()).toEqual(otto);
  });

  it("never sends a name the rules refuse", async () => {
    const { io, shown, sent } = world(() => Promise.reject(new Error("not asked")), otto);
    expect(await renameEmber("x".repeat(30), io)).toEqual({ ok: false, error: "24 characters at most" });
    expect(sent).toEqual([]);
    expect(shown).toEqual([]);
  });

  it("sends null to go back to Your ember", async () => {
    const { io, shown, sent } = world(async () => ({ ok: true, ...defaultEmberName, at: "later" }), otto);
    const outcome = await renameEmber("", io);
    expect(sent).toEqual([null]);
    expect(shown).toEqual(["Your ember", "Your ember"]);
    expect(outcome.ok).toBe(true);
  });
});

describe("the chat's title", () => {
  it("names Main as you named the ember", () => {
    expect(chatTitle({ main: true, name: "Main" }, undefined, "Pip")).toBe("Pip");
    expect(chatTitle({ main: true, name: "Main" }, undefined, undefined)).toBe("Main");
    expect(chatTitle({ main: false, name: "Garden" }, undefined, "Pip")).toBe("Garden");
    expect(chatTitle({ main: false, name: "Garden" }, "Helper", "Pip")).toBe("Helper");
    expect(chatTitle({ main: true }, undefined, undefined)).toBe("Emberhut");
  });
});
