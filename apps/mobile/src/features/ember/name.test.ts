import { describe, expect, it } from "vitest";
import { ApiError, type EmberName, type EmberNameReply } from "@/api";
import {
  chatTitle,
  composerPrompt,
  cleanEmberName,
  customEmberName,
  defaultEmberName,
  emberCalled,
  emberName,
  emberNameLength,
  fallbackGraphemes,
  graphemes,
  leaving,
  nameCount,
  renameEmber,
  shortEmberName,
  takesEdit,
  type RenameIo,
} from "./name";

const pip: EmberName = { name: "Pip", custom: true, at: "2026-11-28T10:00:00.000Z" };

describe("the ember's name in a state", () => {
  it("is yours when you gave one, else Your ember", () => {
    expect(emberName({ emberName: pip })).toBe("Pip");
    expect(customEmberName({ emberName: pip })).toBe("Pip");
    expect(emberName({ emberName: defaultEmberName })).toBe("Your ember");
    expect(customEmberName({ emberName: defaultEmberName })).toBeUndefined();
  });

  it("is called by its name mid-sentence, else your ember", () => {
    expect(emberCalled({ emberName: pip })).toBe("Pip");
    expect(emberCalled({ emberName: defaultEmberName })).toBe("your ember");
    expect(emberCalled(undefined)).toBe("your ember");
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
    for (const bad of [
      "Ju\nniper",
      "Ju\tniper",
      "Ju\u0007niper",
      "Ju\u202Eniper",
      "Ju\u200Bniper",
      "Ju\u2028niper",
      "Ju\u200Dniper",
      "Pip\uFEFF",
    ])
      expect(cleanEmberName(bad)).toEqual({ ok: false, error: "One line of plain text" });
  });

  it("makes each run of spaces inside one plain space", () => {
    expect(cleanEmberName("  Juniper   the\u00A0\u3000Brave ")).toEqual({ ok: true, name: "Juniper the Brave" });
    expect(cleanEmberName("Your \u2002 Ember")).toEqual({ ok: true, name: null });
    expect(emberNameLength(" Pip   Pop ")).toBe(7);
  });

  it("counts an emoji as one, joined, toned or a flag", () => {
    const family = "👨\u200D👩\u200D👧";
    expect(cleanEmberName(`Pip ${family}`)).toEqual({ ok: true, name: `Pip ${family}` });
    for (const one of [family, "👍🏽", "🇯🇴", "❤️", "é"]) expect(emberNameLength(one), one).toBe(1);
    expect(cleanEmberName("🇯🇴".repeat(24))).toEqual({ ok: true, name: "🇯🇴".repeat(24) });
    expect(cleanEmberName(family.repeat(25))).toEqual({ ok: false, error: "24 characters at most" });
  });
});

describe("characters as people see them without Intl.Segmenter", () => {
  it("splits as the segmenter does for names", () => {
    for (const text of [
      "Juniper",
      "Émile 🔥",
      "e\u0301t\u00E9",
      "👨\u200D👩\u200D👧 and 👍🏽",
      "🇯🇴🇸🇦",
      "❤️❤︎",
      "🏴\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
      "t\u200Dt",
    ])
      expect(fallbackGraphemes(text), text).toEqual(graphemes(text));
  });
});

describe("a short name", () => {
  it("is itself when it fits, else its start and an ellipsis", () => {
    expect(shortEmberName("Juniper", 16)).toBe("Juniper");
    expect(shortEmberName("Juniper the Bravest Ember", 16)).toBe("Juniper the Bra…");
    expect(shortEmberName("Juniper the Brave", 12)).toBe("Juniper the…");
    expect(shortEmberName("🇯🇴".repeat(20), 3)).toBe("🇯🇴🇯🇴…");
  });
});

describe("the name field in place", () => {
  it("takes no edit past 24 characters, and counts from 20", () => {
    expect(takesEdit("x".repeat(24))).toBe(true);
    expect(takesEdit("x".repeat(25))).toBe(false);
    expect(takesEdit(`${"x".repeat(24)}   `)).toBe(true);
    expect(nameCount("x".repeat(19))).toBeNull();
    expect(nameCount("x".repeat(21))).toBe("21/24");
  });

  it("closes unchanged, renames when changed, and blank means the default without an error", () => {
    expect(leaving(" Pip ", "Pip")).toEqual({ kind: "close" });
    expect(leaving("Juniper", "Pip")).toEqual({ kind: "rename", typed: "Juniper" });
    expect(leaving("", "Pip")).toEqual({ kind: "rename", typed: "" });
    expect(leaving("", undefined)).toEqual({ kind: "close" });
    expect(leaving("your ember", undefined)).toEqual({ kind: "close" });
    expect(leaving("Ju\nniper", "Pip")).toEqual({ kind: "stay", error: "One line of plain text" });
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
    const { io, shown, now } = world(() => Promise.reject(new ApiError(400, "name: one line of plain text")), pip);
    const outcome = await renameEmber("Juniper", io);
    expect(outcome).toEqual({ ok: false, error: "One line of plain text" });
    expect(shown).toEqual(["Juniper", "Pip"]);
    expect(now()).toEqual(pip);
  });

  it("goes back when the server can't be reached", async () => {
    const { io, now } = world(() => Promise.reject(new ApiError(0, "timed out")), pip);
    expect(await renameEmber("Juniper", io)).toEqual({ ok: false, error: "Can't reach your ember" });
    expect(now()).toEqual(pip);
  });

  it("never sends a name the rules refuse", async () => {
    const { io, shown, sent } = world(() => Promise.reject(new Error("not asked")), pip);
    expect(await renameEmber("x".repeat(30), io)).toEqual({ ok: false, error: "24 characters at most" });
    expect(sent).toEqual([]);
    expect(shown).toEqual([]);
  });

  it("sends null to go back to Your ember", async () => {
    const { io, shown, sent } = world(async () => ({ ok: true, ...defaultEmberName, at: "later" }), pip);
    const outcome = await renameEmber("", io);
    expect(sent).toEqual([null]);
    expect(shown).toEqual(["Your ember", "Your ember"]);
    expect(outcome.ok).toBe(true);
  });
});

describe("the chat's title", () => {
  it("names Main as you named the ember", () => {
    expect(chatTitle({ main: true, name: "Main" }, undefined, "Pip")).toBe("Pip");
    expect(chatTitle({ main: true, name: "Main" }, undefined, undefined)).toBe("Your ember");
    expect(chatTitle({ main: false, name: "Garden" }, undefined, "Pip")).toBe("Garden");
    expect(chatTitle({ main: false, name: "Garden" }, "Helper", "Pip")).toBe("Helper");
    expect(chatTitle({ main: true }, undefined, undefined)).toBe("Your ember");
  });
});

describe("the message box's prompt", () => {
  it("names the ember, cut when long, and the project's room; an agent's room names the agent", () => {
    expect(composerPrompt(undefined, undefined, "Juniper")).toBe("Message Juniper");
    expect(composerPrompt(undefined, "Garden", "Juniper")).toBe("Message Juniper in Garden");
    expect(composerPrompt(undefined, undefined, "your ember")).toBe("Message your ember");
    expect(composerPrompt(undefined, undefined, "Juniper the Bravest Ember")).toBe("Message Juniper the Bra…");
    expect(composerPrompt("Helper", "Garden", "Juniper")).toBe("Message Helper");
  });
});
