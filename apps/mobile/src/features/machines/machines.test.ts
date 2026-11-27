import { describe, expect, it } from "vitest";
import { ApiError, type Machine, type MachineSession, type MachineThresholds } from "@/api";
import { isOlderServer, meters, sessionRow, skipLine, stateTone } from "./machines";

const GB = 1024 ** 3;

const limits: MachineThresholds = {
  minAvailableGb: 4,
  minFreeDiskGb: 15,
  maxLoadPerCore: 1.5,
  alertDiskGb: 10,
  warnDiskPct: 95,
  minCapRoomGb: 2,
  staleSeconds: 60,
  offlineSeconds: 120,
};

const machine = (extra: Partial<Machine> = {}): Machine => ({
  id: "machine-a",
  name: "Studio",
  host: "home-server",
  platform: "linux",
  state: "ok",
  status: "Online · 8 s ago",
  seen: "2026-11-24T16:19:52Z",
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

const byKey = (m: Machine) => Object.fromEntries(meters(m, limits).map((x) => [x.key, x]));

describe("the header", () => {
  it("tones the badge by state", () => {
    expect(stateTone("ok")).toBe("done");
    expect(stateTone("warn")).toBe("warning");
    expect(stateTone("low")).toBe("danger");
    expect(stateTone("stale")).toBe("muted");
    expect(stateTone("offline")).toBe("muted");
    expect(stateTone("unknown")).toBe("muted");
  });

  it("says why a machine takes no new sessions", () => {
    expect(skipLine(machine())).toBeNull();
    expect(skipLine(machine({ placeable: false, skipWhy: "only 2.1 GB memory available (needs 4 GB)" }))).toBe(
      "Not taking new sessions: only 2.1 GB memory available (needs 4 GB)",
    );
    expect(skipLine(machine({ placeable: false }))).toBe("Not taking new sessions");
  });

  it("knows a server on older code by its 404", () => {
    expect(isOlderServer(new ApiError(404, "not found"))).toBe(true);
    expect(isOlderServer(new ApiError(500, "boom"))).toBe(false);
    expect(isOlderServer(new Error("x"))).toBe(false);
  });
});

describe("meters", () => {
  it("has none for a machine that never reported", () => {
    expect(meters(machine({ state: "unknown" }), limits)).toEqual([]);
  });

  it("reads CPU as load on cores and the busy share", () => {
    const cpu = byKey(machine({ cpu: { cores: 32, load1: 6.1, load5: 9.9, load15: 6.4, usage: 0.12 } })).cpu!;
    expect(cpu.value).toBe("load 6.1 / 32 cores · 12%");
    expect(cpu.share).toBe(0.12);
    expect(cpu.level).toBe("ok");
    const busy = byKey(machine({ cpu: { cores: 4, load1: 8, load5: 8, load15: 8, usage: null } })).cpu!;
    expect(busy.value).toBe("load 8.0 / 4 cores");
    expect(busy.share).toBe(1);
    expect(busy.level).toBe("warn");
  });

  it("reads memory as used of total, what's available and pressure only when it isn't normal", () => {
    const memory = { total: 64 * GB, available: 50 * GB, used: 14 * GB, swapTotal: 0, swapUsed: 0, pressure: "normal" as const };
    const fine = byKey(machine({ memory })).memory!;
    expect(fine.value).toBe("14.0 of 64.0 GB");
    expect(fine.detail).toBe("50.0 GB available");
    expect(fine.level).toBe("ok");
    const tight = byKey(machine({ memory: { ...memory, available: 2.1 * GB, pressure: "warn" } })).memory!;
    expect(tight.detail).toBe("2.1 GB available · pressure warn");
    expect(tight.level).toBe("low");
  });

  it("gives each GPU its busy share and memory, with what holds the memory", () => {
    const gpus = [
      {
        index: 0,
        name: "NVIDIA GeForce RTX 5090 Laptop GPU",
        util: 0,
        memUsed: 22 * GB,
        memTotal: 24 * GB,
        temp: 49,
        power: 4,
        procs: [{ pid: 1, name: "llama-server", mem: 19.8 * GB, unit: "llm-server.service", user: "ada" }],
      },
    ];
    const m = byKey(machine({ gpus }));
    expect(m["gpu-0"]!.label).toBe("NVIDIA GeForce RTX 5090 Laptop GPU");
    expect(m["gpu-0"]!.value).toBe("0% busy");
    expect(m["gpu-0"]!.detail).toBe("49°C · 4 W");
    expect(m["vram-0"]!.value).toBe("22.0 of 24.0 GB");
    expect(m["vram-0"]!.level).toBe("warn");
    expect(m["vram-0"]!.lines).toEqual(["llama-server · llm-server.service · 19.8 GB"]);
  });

  it("names the Mac's Apple GPU with its cores and the unified memory it holds", () => {
    const gpus = [
      {
        index: 0,
        name: "Apple M4 Max",
        cores: 32,
        util: 0.44,
        memUsed: 2 * GB,
        memTotal: 36 * GB,
        temp: null,
        power: null,
        shared: true,
        procs: [],
      },
    ];
    const m = byKey(machine({ gpus }));
    expect(m["gpu-0"]!.label).toBe("Apple M4 Max GPU · 32 cores");
    expect(m["gpu-0"]!.value).toBe("44% busy");
    expect(m["vram-0"]!.value).toBe("2.0 of 36.0 GB unified");
  });

  it("reads each disk as used of total and free, warning by the server's thresholds", () => {
    const disk = (free: number) => byKey(machine({ disks: [{ path: "/", total: 200 * GB, free: free * GB }] }))["disk-/"]!;
    expect(disk(50).value).toBe("150.0 of 200.0 GB");
    expect(disk(50).detail).toBe("50.0 GB free");
    expect(disk(50).level).toBe("ok");
    expect(disk(12).level).toBe("warn");
    expect(disk(8).level).toBe("low");
    // 97% full warns even with plenty free.
    const big = byKey(machine({ disks: [{ path: "/home", total: 2000 * GB, free: 58 * GB }] }))["disk-/home"]!;
    expect(big.level).toBe("warn");
  });

  it("shows a battery only when there is one", () => {
    expect(byKey(machine()).battery).toBeUndefined();
    const battery = byKey(machine({ battery: { percent: 75, plugged: true, state: "not charging", minutesLeft: null } })).battery!;
    expect(battery.value).toBe("75%");
    expect(battery.detail).toBe("plugged in · not charging");
    const draining = byKey(machine({ battery: { percent: 15, plugged: false, state: "discharging", minutesLeft: 125 } })).battery!;
    expect(draining.detail).toBe("on battery · discharging · 2 h 5 min left");
    expect(draining.level).toBe("low");
  });

  it("shows the runner's memory cap, low when it has little room left", () => {
    const runner = (used: number) => ({
      sessions: 1,
      queued: 0,
      open: 2,
      claude: 10,
      claudeRss: 2 * GB,
      rss: 0.1 * GB,
      cap: { name: "emberhut-runner.service", used: used * GB, high: 36 * GB, max: 44 * GB, by: "work.slice" },
    });
    const cap = byKey(machine({ runner: runner(27.3) })).cap!;
    expect(cap.value).toBe("27.3 of 44.0 GB");
    expect(cap.detail).toBe("slows at 36.0 GB · set on work.slice");
    expect(cap.level).toBe("ok");
    expect(byKey(machine({ runner: runner(43) })).cap!.level).toBe("low");
    expect(byKey(machine({ runner: { ...runner(1), cap: null } })).cap).toBeUndefined();
  });
});

describe("sessionRow", () => {
  const session = (extra: Partial<MachineSession> = {}): MachineSession => ({
    id: "r-example",
    title: "Emberhut · Live capacity of every machine",
    status: "running",
    running: true,
    doing: "Editing machines.ts",
    account: "bedrock",
    accountName: "AWS Bedrock",
    used5h: null,
    used7d: null,
    limitedUntil: null,
    ...extra,
  });

  it("drops Emberhut's prefix and says what it's doing on which account", () => {
    expect(sessionRow(session())).toEqual({
      title: "Live capacity of every machine",
      subtitle: "Running · Editing machines.ts · AWS Bedrock",
    });
  });

  it("gives the account's use when known", () => {
    const row = sessionRow(session({ account: "dev1", accountName: "Dev 1", used5h: 12.4, used7d: 30, doing: null }));
    expect(row.subtitle).toBe("Running · Dev 1 (5h 12%, 7d 30%)");
  });
});
