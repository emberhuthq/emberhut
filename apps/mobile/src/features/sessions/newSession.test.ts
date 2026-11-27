import { describe, expect, it } from "vitest";
import type { ClaudeAccount, FolderRoots } from "@/api";
import { accountLine, accountsFor, quickFolders, sessionFolder, sessionMachine, wantsBranches } from "./newSession";

describe("new session", () => {
  it("offers your projects then recent folders, once each", () => {
    const roots = {
      machine: "machine-a",
      home: "/home/ada",
      documents: "/home/ada/Documents",
      projects: [{ name: "emberhut", path: "/home/ada/emberhut" }],
      recent: [
        { name: "emberhut", path: "/home/ada/emberhut", git: true, used: "" },
        { name: "", path: "/home/ada/Documents/notes", git: true, used: "" },
      ],
    } as FolderRoots;
    expect(quickFolders(roots)).toEqual([
      { path: "/home/ada/emberhut", name: "emberhut" },
      { path: "/home/ada/Documents/notes", name: "notes" },
    ]);
    expect(quickFolders(undefined)).toEqual([]);
    // The branches asked for are the folder's that is shown and sent, picked or not.
    expect(sessionFolder("", roots)).toBe("/home/ada/emberhut");
    expect(sessionFolder("/home/ada/Documents/notes", roots)).toBe("/home/ada/Documents/notes");
    expect(sessionFolder("", undefined)).toBe("");
    expect(wantsBranches(true, true, sessionFolder("", roots))).toBe(true);
    expect(wantsBranches(true, false, "/x")).toBe(false);
    expect(wantsBranches(true, true, "")).toBe(false);
  });

  it("says how much room an account has", () => {
    expect(accountLine({ id: "a", name: "Personal", used7d: 41.6, used5h: 3 })).toBe("Personal · 42% week · 3% 5 h");
    expect(accountLine({ id: "b", name: "Dev", loggedIn: false, scope: "work" })).toBe("Dev · not logged in · Work only");
    expect(accountLine({ id: "c", name: "X", limitedUntil: "2026-11-24T14:00:00Z" }, Date.parse("2026-11-24T12:00:00Z"))).toMatch(
      /^X · out until /,
    );
  });

  it("starts on the machine picked while the server offers it, else its first", () => {
    const offered = [{ id: "machine-a" }, { id: "machine-b" }];
    expect(sessionMachine(null, offered)).toBe("machine-a");
    expect(sessionMachine("machine-b", offered)).toBe("machine-b");
    expect(sessionMachine("gone", offered)).toBe("machine-a");
    expect(sessionMachine("machine-a", [])).toBeNull();
  });

  it("offers the accounts of the session's machine, and those tied to none", () => {
    const account = (id: string, extra: Partial<ClaudeAccount> = {}): ClaudeAccount => ({ id, name: id, ...extra });
    const all = [
      account("a", { machine: "machine-a" }),
      account("b", { machine: "machine-b" }),
      account("anywhere"),
      account("kept", { machine: "machine-a", scope: "factory" }),
      account("off", { machine: "machine-a", enabled: false }),
    ];
    expect(accountsFor(all, "machine-a").map((a) => a.id)).toEqual(["a", "anywhere"]);
    expect(accountsFor(all, "machine-b").map((a) => a.id)).toEqual(["b", "anywhere"]);
  });
});
