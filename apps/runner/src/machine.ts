// What this machine has left, sampled every 20 s and posted to the server (`POST /api/runner/machine`), which keeps the
// latest and a day of history, marks a machine stale or offline when samples stop, and skips a full machine when it
// places new sessions: CPU load and busy share, memory and its pressure, each GPU and the processes holding its memory,
// free disk, the battery, and this runner's sessions, agent processes and memory cap.
//
// Cheap on purpose: the fast part is file reads and statfs (a few ms); what needs a program (nvidia-smi, ps, pmset)
// runs at most once a minute and never blocks. Units: bytes are bytes, `usage` and `util` are 0-1, battery percent 0-100.
import { execFile } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statfsSync, statSync } from "node:fs";
import { cpus, freemem, homedir, hostname, loadavg, platform, totalmem } from "node:os";
import { inBackground, log } from "./log.ts";
import { SERVER } from "./wire.ts";

const OS = platform();
const MAC = OS === "darwin";
const run = (cmd: string, args: string[], timeout = 8_000) =>
  new Promise<string | null>((done) =>
    execFile(cmd, args, { encoding: "utf8", timeout, maxBuffer: 4 * 1024 * 1024 }, (e, out) => done(e ? null : out)),
  );
const read = (f: string) => {
  try {
    return readFileSync(f, "utf8").trim();
  } catch {
    return null;
  }
};
const num = (s: string | null | undefined): number | null => {
  const n = Number(String(s ?? "").trim());
  return s != null && String(s).trim() !== "" && Number.isFinite(n) ? n : null;
};

let lastCpu: { busy: number; total: number } | null = null;
/** CPU busy share since the last sample, from the per-core times. */
function cpuUse(): number | null {
  let busy = 0,
    total = 0;
  for (const c of cpus()) {
    const t = c.times;
    busy += t.user + t.nice + t.sys + t.irq;
    total += t.user + t.nice + t.sys + t.irq + t.idle;
  }
  const was = lastCpu;
  lastCpu = { busy, total };
  return was && total > was.total ? Math.round(((busy - was.busy) / (total - was.total)) * 1000) / 1000 : null;
}

/** Memory as the server reads it. */
export type Memory = {
  total: number;
  available: number;
  used: number;
  swapTotal: number;
  swapUsed: number;
  pressure: "normal" | "warn" | "critical";
  psi?: { some60: number; full60: number };
};

/** Linux: /proc/meminfo's MemAvailable and the kernel's memory pressure stall (the share of time tasks waited on memory). */
function linuxMemory(): Memory {
  const m: Record<string, number> = {};
  for (const l of (read("/proc/meminfo") ?? "").split("\n")) {
    const x = l.match(/^(\w+):\s+(\d+)/);
    if (x) m[x[1]!] = Number(x[2]) * 1024;
  }
  const p = read("/proc/pressure/memory") ?? "";
  const avg = (k: string) => Number(p.match(new RegExp(`^${k} .*?avg60=([\\d.]+)`, "m"))?.[1] ?? 0);
  const psi = { some60: avg("some"), full60: avg("full") };
  const total = m.MemTotal ?? totalmem(),
    available = m.MemAvailable ?? freemem();
  // full: every task stalled on memory at once. Critical at 10% of the last minute, or under 3% of memory free.
  const pressure =
    psi.full60 >= 10 || available < total * 0.03 ? "critical" : psi.some60 >= 20 || available < total * 0.08 ? "warn" : "normal";
  return {
    total,
    available,
    used: total - available,
    swapTotal: m.SwapTotal ?? 0,
    swapUsed: (m.SwapTotal ?? 0) - (m.SwapFree ?? 0),
    pressure,
    psi,
  };
}
/** macOS: vm_stat's pages it can hand out (free, inactive, purgeable, speculative) and the kernel's own pressure level. */
async function macMemory(): Promise<Memory> {
  const [vm, sys] = await Promise.all([
    run("/usr/bin/vm_stat", []),
    run("/usr/sbin/sysctl", ["-n", "hw.memsize", "kern.memorystatus_vm_pressure_level", "vm.swapusage"]),
  ]);
  const page = Number(vm?.match(/page size of (\d+)/)?.[1] ?? 16384);
  const pages = (k: string) => Number(vm?.match(new RegExp(`${k}:\\s+(\\d+)`))?.[1] ?? 0) * page;
  const [memsize, level, swap] = (sys ?? "").split("\n");
  const total = num(memsize) ?? totalmem();
  const free = vm ? pages("Pages free") + pages("Pages inactive") + pages("Pages purgeable") + pages("Pages speculative") : freemem();
  const available = Math.min(free, total);
  const mb = (k: string) => Number(swap?.match(new RegExp(`${k} = ([\\d.]+)M`))?.[1] ?? 0) * 1024 * 1024;
  // kern.memorystatus_vm_pressure_level: 1 normal, 2 warn, 4 critical.
  const lv = num(level);
  return {
    total,
    available,
    used: total - available,
    swapTotal: mb("total"),
    swapUsed: mb("used"),
    pressure: lv === 4 ? "critical" : lv === 2 ? "warn" : "normal",
  };
}
/** Elsewhere: what Node knows. */
function plainMemory(): Memory {
  const total = totalmem(),
    available = freemem();
  return {
    total,
    available,
    used: total - available,
    swapTotal: 0,
    swapUsed: 0,
    pressure: available < total * 0.03 ? "critical" : available < total * 0.08 ? "warn" : "normal",
  };
}

/** The volumes that matter: / and /home on Linux (one disk counted once), / on macOS, the home folder's drive elsewhere. */
function disks(): { path: string; total: number; free: number }[] {
  const out: { path: string; total: number; free: number }[] = [],
    seen = new Set<number>();
  const paths = MAC ? ["/"] : OS === "linux" ? ["/", "/home"] : [homedir().slice(0, 3)];
  for (const path of paths)
    try {
      const dev = statSync(path).dev;
      if (seen.has(dev)) continue;
      seen.add(dev);
      const s = statfsSync(path);
      out.push({ path, total: s.blocks * s.bsize, free: s.bavail * s.bsize });
    } catch {
      // Not on this machine.
    }
  return out;
}

/** Linux: the runner's cgroup and the tightest memory limit above it (its unit's, or a slice's). */
function cgroup() {
  if (OS !== "linux") return null;
  const path = (read("/proc/self/cgroup") ?? "").match(/^0::(\/.*)$/m)?.[1];
  if (!path) return null;
  const at = (p: string, f: string) => {
    const v = read(`/sys/fs/cgroup${p}/${f}`);
    return v && v !== "max" ? num(v) : null;
  };
  let high: number | null = null,
    max: number | null = null,
    highAt: string | null = null,
    maxAt: string | null = null;
  for (let p = path; p && p !== "/"; p = p.slice(0, p.lastIndexOf("/")) || "/") {
    const h = at(p, "memory.high"),
      m = at(p, "memory.max");
    if (h != null && (high == null || h < high)) {
      high = h;
      highAt = p;
    }
    if (m != null && (max == null || m < max)) {
      max = m;
      maxAt = p;
    }
  }
  // What the cap is measured against: the working set of the cgroup that has it (memory.current less the inactive file
  // cache the kernel takes back first, as container runtimes count it).
  const working = (p: string) => {
    const cur = at(p, "memory.current");
    const inactive = Number((read(`/sys/fs/cgroup${p}/memory.stat`) ?? "").match(/^inactive_file (\d+)$/m)?.[1] ?? 0);
    return cur == null ? null : Math.max(0, cur - inactive);
  };
  const capAt = maxAt ?? highAt,
    name = (p: string | null) => (p ? (p.split("/").pop() ?? null) : null);
  return {
    name: name(path),
    current: at(path, "memory.current"),
    high,
    max,
    highBy: name(highAt),
    maxBy: name(maxAt),
    capUsed: capAt ? working(capAt) : null,
  };
}

type Battery = { percent: number | null; plugged: boolean; state: string; minutesLeft: number | null };
/** Linux's power_supply class; null on a machine with no battery. */
function linuxBattery(): Battery | null {
  const base = "/sys/class/power_supply";
  let list: string[] = [];
  try {
    list = readdirSync(base);
  } catch {
    return null;
  }
  const bat = list.find((d) => read(`${base}/${d}/type`) === "Battery");
  if (!bat) return null;
  const mains = list.filter((d) => read(`${base}/${d}/type`) === "Mains").some((d) => read(`${base}/${d}/online`) === "1");
  const status = read(`${base}/${bat}/status`) ?? "Unknown";
  const power = num(read(`${base}/${bat}/power_now`)),
    energy = num(read(`${base}/${bat}/energy_now`));
  const minutesLeft = status === "Discharging" && power && energy ? Math.round((energy / power) * 60) : null;
  return { percent: num(read(`${base}/${bat}/capacity`)), plugged: mains, state: status.toLowerCase(), minutesLeft };
}
/** macOS's pmset. */
async function macBattery(): Promise<Battery | null> {
  const out = await run("/usr/bin/pmset", ["-g", "batt"]);
  if (!out || !/InternalBattery/.test(out)) return null;
  const pct = num(out.match(/(\d+)%/)?.[1]),
    state =
      out
        .match(/\d+%;\s*([\w ]+?);/)?.[1]
        ?.trim()
        .toLowerCase() ?? "unknown";
  const left = out.match(/(\d+):(\d+) remaining/);
  return {
    percent: pct,
    plugged: /AC Power/.test(out),
    state,
    minutesLeft: left && state === "discharging" ? Number(left[1]) * 60 + Number(left[2]) : null,
  };
}

/** A GPU as the server reads it; `shared`: its memory is the machine's unified memory. */
export type Gpu = {
  index: number;
  name: string;
  util: number | null;
  memUsed: number;
  memTotal: number;
  temp: number | null;
  power: number | null;
  procs: { pid: number; name: string; mem: number; unit: string | null; user: string | null }[];
  cores?: number | null;
  shared?: boolean;
};
const NVSMI = ["/usr/bin/nvidia-smi", "/usr/local/bin/nvidia-smi"].find((p) => existsSync(p)) ?? null;
/** NVIDIA GPUs (nvidia-smi where it is) and the processes holding their memory, with each one's systemd unit. */
async function nvidiaGpus(): Promise<Gpu[]> {
  if (!NVSMI) return [];
  const [g, p] = await Promise.all([
    run(NVSMI, [
      "--query-gpu=index,uuid,name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw",
      "--format=csv,noheader,nounits",
    ]),
    run(NVSMI, ["--query-compute-apps=gpu_uuid,pid,process_name,used_memory", "--format=csv,noheader,nounits"]),
  ]);
  if (!g) return [];
  const MiB = 1024 * 1024;
  const list = g
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((l) => {
      const [index, uuid, name, util, used, total, temp, power] = l.split(",").map((x) => x.trim());
      const u = num(util);
      const gpu: Gpu = {
        index: Math.trunc(Number(index)),
        name: name ?? "GPU",
        util: u != null ? u / 100 : null,
        memUsed: (num(used) ?? 0) * MiB,
        memTotal: (num(total) ?? 0) * MiB,
        temp: num(temp),
        power: num(power),
        procs: [],
      };
      return { uuid, gpu };
    });
  for (const l of (p ?? "").trim().split("\n").filter(Boolean)) {
    const [uuid, pid, pname, mem] = l.split(",").map((x) => x.trim());
    const at = list.find((x) => x.uuid === uuid) ?? list[0];
    if (!at || !pid) continue;
    const unit = (read(`/proc/${pid}/cgroup`) ?? "").match(/\/([^/]+\.(?:service|scope))\s*$/m)?.[1] ?? null;
    let user: string | null = null;
    try {
      const uid = statSync(`/proc/${pid}`).uid;
      user = uid === process.getuid?.() ? (process.env.USER ?? String(uid)) : String(uid);
    } catch {
      // Gone.
    }
    at.gpu.procs.push({ pid: Math.trunc(Number(pid)), name: (pname ?? "").split("/").pop() ?? "", mem: (num(mem) ?? 0) * MiB, unit, user });
  }
  return list.map((x) => x.gpu);
}

/**
 * Apple silicon's GPU, from its IOAccelerator entry (no root needed): "model", "gpu-core-count", and in its
 * PerformanceStatistics "Device Utilization %" and "In use system memory" (its share of the unified memory).
 */
export function parseIoreg(text: string, total: number): Gpu[] {
  return text
    .split(/^\+-o /m)
    .filter((b) => /"PerformanceStatistics"/.test(b))
    .map((b, index) => {
      const n = (re: RegExp) => num(b.match(re)?.[1]);
      const util = n(/"Device Utilization %"\s*=\s*(\d+)/);
      return {
        index,
        name: b.match(/^\s*"model" = "([^"]+)"/m)?.[1] ?? b.match(/^(\w+)/)?.[1] ?? "Apple GPU",
        cores: n(/^\s*"gpu-core-count" = (\d+)/m),
        util: util != null ? Math.min(util, 100) / 100 : null,
        memUsed: n(/"In use system memory"\s*=\s*(\d+)/) ?? 0,
        memTotal: total,
        temp: null,
        power: null,
        shared: true,
        procs: [],
      };
    });
}
async function appleGpus(): Promise<Gpu[]> {
  const out = await run("/usr/sbin/ioreg", ["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"]);
  return out ? parseIoreg(out, totalmem()) : [];
}

/** The agent's processes on the whole machine and their memory. */
async function agents(): Promise<{ count: number; rss: number } | null> {
  if (OS === "win32") return null;
  const out = await run("/bin/ps", MAC ? ["-axo", "rss=,comm="] : ["-eo", "rss=,comm="]);
  if (!out) return null;
  let n = 0,
    rss = 0;
  for (const l of out.split("\n")) {
    const m = l.trim().match(/^(\d+)\s+(.+)$/);
    if (m && /(^|\/)claude$/.test(m[2]!.trim())) {
      n++;
      rss += Number(m[1]) * 1024;
    }
  }
  return { count: n, rss };
}

let slow: { at: number; gpus: Gpu[]; agents: { count: number; rss: number } | null; battery: Battery | null } = {
  at: 0,
  gpus: [],
  agents: null,
  battery: null,
};
/** The slow parts, at most once a minute (the first sample has them). */
async function slowParts() {
  if (Date.now() - slow.at < 55_000) return slow;
  const [g, c, b] = await Promise.all([
    OS === "linux" ? nvidiaGpus().catch(() => []) : Promise.resolve([]),
    agents().catch(() => null),
    MAC ? macBattery().catch(() => null) : Promise.resolve(OS === "linux" ? linuxBattery() : null),
  ]);
  slow = { at: Date.now(), gpus: g, agents: c, battery: b };
  return slow;
}

/** The runner's sessions, as the sample carries them. */
export type Sessions = { running: number; open: number; queued: number; list: Record<string, unknown>[] };

/** One sample of this machine. */
export async function sample(machine: string, sessions: Sessions): Promise<Record<string, unknown>> {
  const t0 = performance.now(),
    c0 = process.cpuUsage();
  const [memory, s, apple] = await Promise.all([
    MAC ? macMemory() : Promise.resolve(OS === "linux" ? linuxMemory() : plainMemory()),
    slowParts(),
    MAC ? appleGpus().catch(() => []) : Promise.resolve(null),
  ]);
  const [load1, load5, load15] = loadavg();
  const c = process.cpuUsage(c0);
  return {
    machine,
    at: new Date().toISOString(),
    host: hostname(),
    platform: OS,
    cpu: { cores: cpus().length, load1, load5, load15, usage: cpuUse() },
    memory,
    gpus: apple ?? s.gpus,
    disks: disks(),
    battery: s.battery,
    runner: {
      pid: process.pid,
      rss: process.memoryUsage.rss(),
      cgroup: cgroup(),
      sessions: sessions.running,
      queued: sessions.queued,
      open: sessions.open,
      claude: s.agents?.count ?? null,
      claudeRss: s.agents?.rss ?? null,
    },
    sessions: sessions.list,
    // What this sample cost here, so a slow one shows.
    cost: { ms: Math.round((performance.now() - t0) * 10) / 10, cpuMs: Math.round((c.user + c.system) / 100) / 10 },
  };
}

/** Samples the machine every `every` ms and posts each to the server; one that doesn't arrive is dropped (the next is newer). */
export function startSampling(o: {
  server: () => string | null;
  token: () => string | null;
  machine: string;
  sessions: () => Sessions;
  every: number;
  fetch?: typeof fetch;
}): { latest: () => Record<string, unknown> | null; now: () => Promise<Record<string, unknown>>; stop: () => void } {
  let latest: Record<string, unknown> | null = null,
    failing = false;
  const tick = async () => {
    const server = o.server(),
      token = o.token();
    try {
      latest = await sample(o.machine, o.sessions());
      if (!server || !token) return;
      const r = await (o.fetch ?? fetch)(`${server}${SERVER.machine}`, {
        method: "POST",
        body: JSON.stringify(latest),
        signal: AbortSignal.timeout(10_000),
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      }).catch(() => null);
      if (!r?.ok && !failing)
        log(`machine sample not taken by the server (${r ? r.status : "unreachable"}); trying again every ${o.every / 1000} s`);
      if (r?.ok && failing) log("machine samples reach the server again");
      failing = !r?.ok;
    } catch (e) {
      log("machine sample failed:", e instanceof Error ? e.message : String(e));
    }
  };
  cpuUse(); // the first busy share needs a starting point
  const first = setTimeout(() => inBackground("machine sample", tick()), 3_000);
  first.unref();
  const every = setInterval(() => inBackground("machine sample", tick()), o.every);
  every.unref();
  return {
    latest: () => latest,
    now: () => sample(o.machine, o.sessions()),
    stop: () => {
      clearTimeout(first);
      clearInterval(every);
    },
  };
}
