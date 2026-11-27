import type { Instant } from "./common";

/** Every size here is in bytes. */
export type Bytes = number;

/**
 * ok: fine. warn: something to look at, still takes work. low: too full, new sessions aren't placed on it. stale: no
 * sample for over `staleSeconds` (still placed). offline: none for over `offlineSeconds` (not placed). unknown: never
 * reported (an older runner).
 */
export type MachineState = "ok" | "warn" | "low" | "stale" | "offline" | "unknown";

export interface MachineWarning {
  level: "warn" | "low" | "critical";
  text: string;
}

export interface MachineCpu {
  cores: number;
  load1: number;
  load5: number;
  load15: number;
  /** The busy share, 0…1. */
  usage: number | null;
}

export interface MachineMemory {
  total: Bytes;
  available: Bytes;
  used: Bytes;
  swapTotal: Bytes;
  swapUsed: Bytes;
  pressure: "normal" | "warn" | "critical";
}

export interface GpuProcess {
  pid: number;
  name: string;
  mem: Bytes;
  /** The systemd unit holding it. */
  unit: string | null;
  user: string | null;
}

export interface MachineGpu {
  index: number;
  name: string;
  /** 0…1. */
  util: number | null;
  memUsed: Bytes;
  memTotal: Bytes;
  /** °C. */
  temp: number | null;
  /** Watts. */
  power: number | null;
  procs: GpuProcess[];
  /** Apple Silicon's GPU cores (a Mac's). */
  cores?: number | null;
  /** Its memory is the machine's unified memory: memTotal is all of it. */
  shared?: boolean;
}

export interface MachineDisk {
  path: string;
  total: Bytes;
  free: Bytes;
}

export interface MachineBattery {
  percent: number | null;
  plugged: boolean;
  /** charging, discharging, not charging, full, charged, unknown, … */
  state: string;
  minutesLeft: number | null;
}

export interface RunnerCap {
  name: string;
  used: Bytes | null;
  high: Bytes | null;
  max: Bytes | null;
  /** The unit or slice whose limit it is. */
  by: string | null;
}

export interface MachineRunner {
  sessions: number;
  queued: number;
  open: number;
  claude: number | null;
  claudeRss: Bytes | null;
  rss: Bytes;
  cap: RunnerCap | null;
}

export interface MachineSession {
  id: string;
  title: string;
  status: string;
  running: boolean;
  doing: string | null;
  account: string | null;
  accountName: string | null;
  /** 0…100. */
  used5h: number | null;
  used7d: number | null;
  limitedUntil: Instant | null;
}

/** One 5-minute point of the last 24 h. */
export interface MachinePoint {
  at: Instant;
  cpu: number | null;
  load: number | null;
  memAvail: Bytes | null;
  gpu: number | null;
  vram: Bytes | null;
  diskFree: Bytes | null;
}

/** For "unknown", cpu, memory, runner and battery are null and the lists empty; offline and stale keep the last numbers. */
export interface Machine {
  id: string;
  name: string;
  host: string | null;
  platform: string | null;
  state: MachineState;
  /** The state in words: "Online · 12 s ago". */
  status: string;
  seen: Instant | null;
  age: number | null;
  /** Whether new sessions go to it, and why not. */
  placeable: boolean;
  skipWhy: string | null;
  warnings: MachineWarning[];
  cpu: MachineCpu | null;
  memory: MachineMemory | null;
  gpus: MachineGpu[];
  disks: MachineDisk[];
  battery: MachineBattery | null;
  runner: MachineRunner | null;
  sessions: MachineSession[];
  /** Oldest first. */
  history: MachinePoint[];
}

export interface MachineThresholds {
  minAvailableGb: number;
  minFreeDiskGb: number;
  maxLoadPerCore: number;
  alertDiskGb: number;
  warnDiskPct: number;
  minCapRoomGb: number;
  staleSeconds: number;
  offlineSeconds: number;
}

/** GET /api/machines: each runner machine's live capacity, in the server's order. */
export interface MachinesPage {
  at: Instant;
  thresholds: MachineThresholds;
  machines: Machine[];
}
