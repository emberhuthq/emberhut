import { describe, expect, it } from "vitest";
import type { ClaudeAccount, Machine } from "@/api";
import { builtInTheme } from "@/theme/builtIn";
import { contrast, paletteFrom } from "@/theme/colors";
import {
  accountsOn,
  ago,
  asOf,
  countLine,
  hutKind,
  kindIcon,
  pillLine,
  resourcesLine,
  room,
  sessionsLine,
  tinted,
  track,
  updatedLine,
} from "./huts";

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
  it("says it once, in the pill, with how long an asleep hut has been quiet", () => {
    expect(pillLine(machine({ state: "warn" }), now)).toBe("Online");
    expect(pillLine(machine({ state: "offline", seen: "2026-11-28T10:00:00Z" }), now)).toBe("Asleep · 2 h ago");
    expect(pillLine(machine({ state: "stale", seen: null, age: 90 }), now)).toBe("Quiet · 1 min ago");
    expect(pillLine(machine({ state: "unknown" }), now)).toBe("Not reported yet");
    expect(ago(30)).toBe("just now");
    expect(ago(3 * 86_400)).toBe("3 d ago");
  });

  it("dates an asleep hut's figures, and the page's last update", () => {
    expect(asOf(machine({ state: "offline", seen: "2026-11-28T10:00:00Z" }), now)).toBe("as of 2 h ago");
    expect(asOf(machine(), now)).toBeNull();
    expect(updatedLine(new Date("2026-11-28T11:59:50Z"), now)).toBe("Updated just now");
    expect(updatedLine(new Date("2026-11-28T11:57:00Z"), now)).toBe("Updated 3 min ago");
  });
});

describe("what runs there", () => {
  it("counts its sessions, the working ones and the queued turns", () => {
    expect(sessionsLine(machine())).toBe("None");
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
    expect(sessionsLine(machine({ runner, sessions: [session("a", true), session("b", false), session("c", false)] }))).toBe(
      "3 open · 1 working · 2 queued",
    );
  });

  it("reads its resources in whole numbers, the GPU as a state", () => {
    const gpu = (util: number | null) => ({
      index: 0,
      name: "Example GPU",
      util,
      memUsed: 0,
      memTotal: GB,
      temp: null,
      power: null,
      procs: [],
    });
    const hut = (gpus: ReturnType<typeof gpu>[]) =>
      machine({
        cpu: { cores: 8, load1: 1, load5: 1, load15: 1, usage: 0.12 },
        memory: { total: 64 * GB, available: 46.2 * GB, used: 18 * GB, swapTotal: 0, swapUsed: 0, pressure: "normal" },
        gpus,
      });
    expect(resourcesLine(hut([gpu(0.01)]))).toBe("CPU 12% · RAM 18 / 64 GB · GPU idle");
    expect(resourcesLine(hut([gpu(0.1), gpu(0.45)]))).toBe("CPU 12% · RAM 18 / 64 GB · GPUs 45%");
    expect(resourcesLine(hut([gpu(null)]))).toBe("CPU 12% · RAM 18 / 64 GB");
    expect(resourcesLine(machine())).toBeNull();
  });
});

describe("an account's room", () => {
  it("is what the higher known use leaves, said as left", () => {
    expect(room(account({ used5h: 28, used7d: 10 }), now)).toEqual({
      value: "72% left",
      detail: "5-hour 72% left · week 90% left",
      share: 0.72,
      tone: "fine",
    });
  });

  it("is red at 15% or less, amber to 40%, and says when it comes back", () => {
    const tone = (used: number) => room(account({ used5h: used }), now).tone;
    expect([tone(85), tone(84), tone(60), tone(59)]).toEqual(["out", "low", "low", "fine"]);
    const low = room(account({ used5h: 91, used7d: 40, resets5h: "2026-11-28T13:20:00Z", resets7d: "2026-12-01T00:00:00Z" }), now);
    expect(low.value).toBe("9% left · back in 1 h 20 m");
    expect(room(account({ used5h: 10, resets5h: "2026-11-28T13:20:00Z" }), now).value).toBe("90% left");
  });

  it("says an unknown 5-hour use is not reported, and the room is from the week only", () => {
    const r = room(account({ used5h: null, used7d: 40 }), now);
    expect(r.value).toBe("60% left · week only");
    expect(r.detail).toBe("5-hour not reported · week 60% left");
  });

  it("says an account with no use known hasn't been checked yet, never 100% left", () => {
    expect(room(account(), now)).toEqual({ value: "Not checked yet", detail: "", share: null, tone: "unknown" });
  });

  it("says how old an asleep hut's figures are", () => {
    expect(room(account({ used5h: 28, used7d: 10, checked: "2026-11-28T10:00:00Z" }), now, true).detail).toBe(
      "5-hour 72% left · week 90% left · checked 2 h ago",
    );
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

describe("the colours", () => {
  it("make the pill's words and the bar's edge readable in both appearances", () => {
    for (const scheme of ["light", "dark"] as const) {
      const colors = paletteFrom(builtInTheme.tokens[scheme], scheme);
      for (const ink of [colors.done, colors.warning, colors.danger, colors.textSecondary]) {
        const { fill, text } = tinted(ink, colors.surface);
        expect(contrast(text, fill)).toBeGreaterThanOrEqual(4.5);
      }
      expect(contrast(track(colors.textSecondary, colors.surface).line, colors.surface)).toBeGreaterThanOrEqual(3);
    }
  });
});
