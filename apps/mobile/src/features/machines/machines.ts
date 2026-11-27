import { ApiError, type Machine, type MachineSession, type MachineState, type MachineThresholds, type MachineWarning } from "@/api";
import { bytesPerGb, gb, gbOf, percent, share } from "@/format/size";
import { parseInstant, span, weekdayClock } from "@/format/time";
import { sessionStatusText, shortTitle } from "@/features/sessions/sessionList";

// The Machines page's words and bars: each runner machine's capacity as GET /api/machines gives it.

/** How a bar or a line reads: fine, worth a look, too full. */
export type Level = "ok" | "warn" | "low";

/** One bar: what, how much in words, how full (0…1), and lines under it. */
export interface Meter {
  key: string;
  label: string;
  value: string;
  share: number;
  level: Level;
  detail?: string;
  /** More lines under the bar: the processes holding a GPU's memory. */
  lines?: string[];
}

export const olderServerText = "Emberhut's server is on older code: this comes with its next update";

/** The server answers 404 when it doesn't know /api/machines yet. */
export const isOlderServer = (error: unknown): boolean => error instanceof ApiError && error.status === 404;

/** The state badge's tone: ok green, warn orange, low red, the rest grey. */
export function stateTone(state: MachineState): "done" | "warning" | "danger" | "muted" {
  if (state === "ok") return "done";
  if (state === "warn") return "warning";
  if (state === "low") return "danger";
  return "muted";
}

export const warningLevel = (warning: MachineWarning): Level => (warning.level === "warn" ? "warn" : "low");

/** "Not taking new sessions: …" when new work isn't placed on it. */
export function skipLine(machine: Machine): string | null {
  if (machine.placeable) return null;
  return machine.skipWhy ? `Not taking new sessions: ${machine.skipWhy}` : "Not taking new sessions";
}

/** The machine's bars, in the page's order: CPU, memory, each GPU, each disk, the battery, the runner's memory cap. */
export function meters(machine: Machine, limits: MachineThresholds): Meter[] {
  const out: Meter[] = [];
  const { cpu, memory, battery } = machine;
  if (cpu) {
    const perCore = cpu.cores > 0 ? cpu.load1 / cpu.cores : 0;
    out.push({
      key: "cpu",
      label: "CPU",
      value: [`load ${cpu.load1.toFixed(1)} / ${cpu.cores} cores`, cpu.usage !== null ? percent(cpu.usage) : null]
        .filter(Boolean)
        .join(" · "),
      share: cpu.usage ?? Math.min(perCore, 1),
      level: perCore > limits.maxLoadPerCore ? "warn" : "ok",
    });
  }
  if (memory) {
    const short = memory.available < limits.minAvailableGb * bytesPerGb;
    out.push({
      key: "memory",
      label: "Memory",
      value: gbOf(memory.used, memory.total),
      share: share(memory.used, memory.total),
      level: short || memory.pressure === "critical" ? "low" : memory.pressure === "warn" ? "warn" : "ok",
      detail: [`${gb(memory.available)} available`, memory.pressure !== "normal" ? `pressure ${memory.pressure}` : null]
        .filter(Boolean)
        .join(" · "),
    });
  }
  const severalGpus = machine.gpus.length > 1;
  for (const gpu of machine.gpus) {
    // An Apple Silicon GPU: "Apple M4 Max GPU · 32 cores", holding part of the unified memory.
    const own = [gpu.shared ? `${gpu.name} GPU` : gpu.name, gpu.cores ? `${gpu.cores} cores` : null].filter(Boolean).join(" · ");
    const name = severalGpus ? `GPU ${gpu.index}: ${own}` : own;
    out.push({
      key: `gpu-${gpu.index}`,
      label: name,
      value: gpu.util !== null ? `${percent(gpu.util)} busy` : "busy share unknown",
      share: gpu.util ?? 0,
      level: "ok",
      detail: [gpu.temp !== null ? `${Math.round(gpu.temp)}°C` : null, gpu.power !== null ? `${Math.round(gpu.power)} W` : null]
        .filter(Boolean)
        .join(" · "),
    });
    const vram = share(gpu.memUsed, gpu.memTotal);
    out.push({
      key: `vram-${gpu.index}`,
      label: severalGpus ? `GPU ${gpu.index} memory` : "GPU memory",
      value: gpu.shared ? `${gbOf(gpu.memUsed, gpu.memTotal)} unified` : gbOf(gpu.memUsed, gpu.memTotal),
      share: vram,
      level: vram > 0.9 ? "warn" : "ok",
      lines: gpu.procs.map((p) => [p.name, p.unit, gb(p.mem)].filter(Boolean).join(" · ")),
    });
  }
  for (const disk of machine.disks) {
    const used = disk.total - disk.free;
    const full = share(used, disk.total);
    const level: Level =
      disk.free < limits.alertDiskGb * bytesPerGb
        ? "low"
        : disk.free < limits.minFreeDiskGb * bytesPerGb || full * 100 >= limits.warnDiskPct
          ? "warn"
          : "ok";
    out.push({
      key: `disk-${disk.path}`,
      label: `Disk ${disk.path}`,
      value: gbOf(used, disk.total),
      share: full,
      level,
      detail: `${gb(disk.free)} free`,
    });
  }
  if (battery) {
    const left =
      battery.minutesLeft !== null && battery.minutesLeft > 0 ? `${span(new Date(0), new Date(battery.minutesLeft * 60_000))} left` : null;
    out.push({
      key: "battery",
      label: "Battery",
      value: battery.percent !== null ? `${Math.round(battery.percent)}%` : "level unknown",
      share: (battery.percent ?? 0) / 100,
      level: !battery.plugged && battery.percent !== null && battery.percent < 20 ? "low" : "ok",
      detail: [battery.plugged ? "plugged in" : "on battery", battery.state, left].filter(Boolean).join(" · "),
    });
  }
  const cap = machine.runner?.cap;
  if (cap) {
    const limit = cap.max ?? cap.high;
    const room = limit !== null && cap.used !== null ? limit - cap.used : null;
    out.push({
      key: "cap",
      label: "Runner's memory cap",
      value:
        limit !== null && cap.used !== null
          ? gbOf(cap.used, limit)
          : limit !== null
            ? `${gb(limit)} cap`
            : cap.used !== null
              ? `${gb(cap.used)} used, no cap`
              : "no cap",
      share: limit !== null && cap.used !== null ? share(cap.used, limit) : 0,
      level: room !== null && room < limits.minCapRoomGb * bytesPerGb ? "low" : "ok",
      detail: [cap.max !== null && cap.high !== null ? `slows at ${gb(cap.high)}` : null, cap.by ? `set on ${cap.by}` : cap.name]
        .filter(Boolean)
        .join(" · "),
    });
  }
  return out;
}

/** A session on the machine: its title without "Emberhut · ", and what it's doing on which account. */
export function sessionRow(session: MachineSession, now: Date = new Date()): { title: string; subtitle: string } {
  const account = session.accountName ?? session.account;
  const use = [
    session.used5h !== null ? `5h ${Math.round(session.used5h)}%` : null,
    session.used7d !== null ? `7d ${Math.round(session.used7d)}%` : null,
  ].filter(Boolean);
  const until = parseInstant(session.limitedUntil);
  const limited = until && until > now ? `limited until ${weekdayClock(until)}` : null;
  return {
    title: shortTitle(session.title) || session.id,
    subtitle: [
      sessionStatusText(session.status),
      session.doing,
      account ? (use.length > 0 ? `${account} (${use.join(", ")})` : account) : null,
      limited,
    ]
      .filter(Boolean)
      .join(" · "),
  };
}
