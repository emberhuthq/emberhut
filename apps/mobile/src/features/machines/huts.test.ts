import { describe, expect, it } from "vitest";
import type { ClaudeAccount, Machine } from "@/api";
import { accountsOn, ago, countLine, hutKind, kindIcon, presenceLine, resourcesLine, room, runningLine } from "./huts";

const GB = 1024 ** 3;
const now = new Date("2026-11-28T12:00:00Z");

const machine = (extra: Partial<Machine> = {}): Machine => ({
  id: "studio",
  name: "Studio",
  host: null,
  platform: "linux",
  state: "ok",
  status: "Online · 8 s ago",
  seen: "2026-11-28T11:59:52Z",
  age: 8,
  placeable: true,
  skipWhy: null,
  warnings: [],
  cpu: null,
  memory: null,
  gpus: [],
  disks: [],
  battery: null,
  runner: null,
  sessions: [],
  history: [],
  ...extra,
});

const account = (extra: Partial<ClaudeAccount> = {}): ClaudeAccount => ({
  id: "a",
  name: "Work",
  machine: "studio",
  loggedIn: true,
  ...extra,
});

describe("a hut's kind", () => {
  it("comes from what its runner reports, a server when its name says so", () => {
    expect(hutKind(machine({ platform: "darwin" }))).toBe("mac");
    expect(hutKind(machine())).toBe("linux");
    expect(hutKind(machine({ platform: "win32" }))).toBe("windows");
    expect(hutKind(machine({ host: "build-server.lan" }))).toBe("server");
    expect(hutKind(machine({ platform: "cloud" }))).toBe("cloud");
    expect(hutKind(machine({ platform: null }))).toBe("computer");
  });

  it("draws a laptop for one with a battery", () => {
    expect(kindIcon("mac", true)).toBe("laptop-outline");
    expect(kindIcon("mac", false)).toBe("desktop-outline");
    expect(kindIcon("server", false)).toBe("server-outline");
  });
});

describe("online or asleep", () => {
  it("says an offline hut is asleep and when it was last seen", () => {
    expect(presenceLine(machine({ state: "warn" }), now)).toBe("Online");
    expect(presenceLine(machine({ state: "offline", seen: "2026-11-28T10:00:00Z" }), now)).toBe("Asleep · last seen 2 h ago");
    expect(presenceLine(machine({ state: "stale", seen: null, age: 90 }), now)).toBe("Quiet · last seen 1 min ago");
    expect(presenceLine(machine({ state: "unknown" }), now)).toBe("Not reported yet");
    expect(ago(30)).toBe("just now");
    expect(ago(3 * 86_400)).toBe("3 d ago");
  });
});

describe("what runs there", () => {
  it("counts its sessions, the running ones and the queued turns", () => {
    expect(runningLine(machine())).toBe("Nothing running");
    const session = (id: string, running: boolean) => ({
      id,
      title: id,
      status: "idle",
      running,
      doing: null,
      account: null,
      accountName: null,
      used5h: null,
      used7d: null,
      limitedUntil: null,
    });
    const runner = { sessions: 1, queued: 2, open: 3, claude: null, claudeRss: null, rss: 0, cap: null };
    expect(runningLine(machine({ runner, sessions: [session("a", true), session("b", false), session("c", false)] }))).toBe(
      "3 sessions · 1 running · 2 queued",
    );
  });

  it("reads its resources in one line", () => {
    const hut = machine({
      cpu: { cores: 8, load1: 1, load5: 1, load15: 1, usage: 0.12 },
      memory: { total: 64 * GB, available: 46 * GB, used: 18 * GB, swapTotal: 0, swapUsed: 0, pressure: "normal" },
    });
    expect(resourcesLine(hut)).toBe("CPU 12% · Memory 18.0 of 64.0 GB");
    expect(resourcesLine(machine())).toBeNull();
  });
});

describe("an account's room", () => {
  it("is what the higher known use leaves", () => {
    expect(room(account({ used5h: 28, used7d: 10 }), now)).toEqual({
      value: "72% left",
      detail: "5-hour 28% used · week 10% used",
      share: 0.72,
      tone: "fine",
    });
    expect(room(account({ used5h: 91, used7d: 40 }), now)).toMatchObject({ value: "9% left", tone: "low" });
  });

  it("says an unknown 5-hour use is not reported, never 0% used", () => {
    const r = room(account({ used5h: null, used7d: 40 }), now);
    expect(r.value).toBe("60% left");
    expect(r.detail).toBe("5-hour not reported · week 40% used");
    expect(r.detail).not.toContain("5-hour 0%");
  });

  it("says an account with no use known hasn't been checked yet, never 100% left", () => {
    const r = room(account(), now);
    expect(r).toEqual({ value: "Not checked yet", detail: "", share: null, tone: "unknown" });
  });

  it("says why an account can't work", () => {
    expect(room(account({ loggedIn: false }), now).value).toBe("Not logged in");
    expect(room(account({ enabled: false }), now).value).toBe("Off");
    expect(room(account({ used5h: 100, limitedUntil: "2026-11-28T18:00:00Z" }), now).value).toMatch(/^Out until \d\d:\d\d$/);
    expect(room(account({ used5h: 20, limitedUntil: "2026-11-28T08:00:00Z" }), now).value).toBe("80% left");
  });

  it("lists the accounts kept on the hut, an unplaced one on the Linux runner", () => {
    const accounts = [
      account(),
      account({ id: "b", name: "Home", machine: "runner" }),
      account({ id: "c", name: "Unplaced", machine: null }),
    ];
    expect(accountsOn(accounts, machine()).map((a) => a.name)).toEqual(["Work"]);
    expect(accountsOn(accounts, machine({ id: "runner" })).map((a) => a.name)).toEqual(["Home", "Unplaced"]);
    expect(accountsOn(undefined, machine())).toEqual([]);
  });
});

describe("the page's line", () => {
  it("counts the huts and the asleep ones", () => {
    expect(countLine([machine()])).toBe("1 hut");
    expect(countLine([machine(), machine({ id: "b", state: "offline" })])).toBe("2 huts · 1 asleep");
  });
});
