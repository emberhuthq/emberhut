// The machine's background agent runs for the server's Background jobs page (`GET /jobs`): `codex exec` runs and
// `claude -p` sessions, each with its age, CPU time, memory, folder and last activity (the run's own log, else the
// newest file in its folder), and on macOS the launchd jobs whose labels match the configured pattern. Plain `ps`, so
// the same on Linux and macOS. Read-only: stopping one is `POST /jobs/stop`, and only ever one of these runs.
import { execFile } from "node:child_process";
import {
  closeSync,
  type Dirent,
  existsSync,
  fstatSync,
  openSync,
  readdirSync,
  readFileSync,
  readlinkSync,
  readSync,
  statSync,
} from "node:fs";
import { homedir, platform } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";

const sh = promisify(execFile);
const mac = platform() === "darwin";

/** What a run did last: its own words, or the file it last changed. */
export type Last = { text: string; at: string | null; from: "codex" | "claude" | "log" | "file" };
/** A process, and when it is a run, which kind. */
export type Proc = {
  pid: number;
  ppid: number;
  age: number;
  cpuTime: number;
  rss: number;
  args: string;
  kind: "codex" | "claude" | null;
  cwd?: string | null;
  unit?: string | null;
  rollout?: string | null;
};

/** `ps`'s times ("1-02:03:04", "02:03:04", "03:04", "03:04.50") in seconds. */
export const secs = (s: string): number => {
  const [d, rest] = s.includes("-") ? s.split("-") : ["0", s];
  return Number(d) * 86_400 + (rest ?? "").split(":").reduce((a, x) => a * 60 + Number(x), 0);
};
/** A background run: `codex exec …` or `claude -p …` (not an interactive session, not the runner's own SDK sessions). */
export const kindOf = (args: string): Proc["kind"] =>
  /^(\S*\/)?codex\s+exec\b/.test(args) ? "codex" : /^(\S*\/)?claude\s+(.*\s)?(-p|--print)(\s|$)/.test(args) ? "claude" : null;

/** Every process, from one `ps`. */
export async function procs(): Promise<Proc[]> {
  if (platform() === "win32") return [];
  const { stdout } = await sh(
    "ps",
    mac ? ["-axww", "-o", "pid=,ppid=,etime=,time=,rss=,command="] : ["-eww", "-o", "pid=,ppid=,etime=,time=,rss=,args="],
    {
      maxBuffer: 64 << 20,
    },
  );
  return stdout
    .split("\n")
    .map((l) => l.trim().match(/^(\d+)\s+(\d+)\s+(\S+)\s+(\S+)\s+(\d+)\s+(.*)$/))
    .filter((m): m is RegExpMatchArray => !!m)
    .map((m) => ({
      pid: +m[1]!,
      ppid: +m[2]!,
      age: secs(m[3]!),
      cpuTime: secs(m[4]!),
      rss: +m[5]! * 1024,
      args: m[6]!.slice(0, 600),
      kind: kindOf(m[6]!),
    }));
}

/** The runs, with the folder each works in and (Linux) the systemd unit it lives in. */
export async function runs(all?: Proc[]): Promise<Proc[]> {
  const list = (all ?? (await procs())).filter((p) => p.kind);
  if (!list.length) return list;
  if (mac) {
    // One lsof for all of them: each one's folder and Codex's rollout file.
    const out = await sh("lsof", ["-a", "-p", list.map((p) => p.pid).join(","), "-Fpfn"], { maxBuffer: 16 << 20 }).then(
      (r) => r.stdout,
      (e: { stdout?: string }) => e.stdout ?? "",
    );
    let pid = 0,
      fd = "";
    for (const l of out.split("\n")) {
      const p = list.find((x) => x.pid === pid);
      if (l[0] === "p") pid = +l.slice(1);
      else if (l[0] === "f") fd = l.slice(1);
      else if (l[0] === "n" && p) {
        if (fd === "cwd") p.cwd = l.slice(1);
        else if (/\/\.codex\/sessions\/.*\.jsonl$/.test(l)) p.rollout = l.slice(1);
      }
    }
    return list;
  }
  for (const p of list) {
    try {
      p.cwd = readlinkSync(`/proc/${p.pid}/cwd`);
    } catch {
      p.cwd = null;
    }
    try {
      p.unit = readFileSync(`/proc/${p.pid}/cgroup`, "utf8").match(/\/([^/\n]+\.(service|scope))\s*$/m)?.[1] ?? null;
    } catch {
      p.unit = null;
    }
    if (p.kind === "codex") p.rollout = rolloutOf(p.pid);
  }
  return list;
}

function rolloutOf(pid: number): string | null {
  try {
    for (const fd of readdirSync(`/proc/${pid}/fd`)) {
      let f = "";
      try {
        f = readlinkSync(`/proc/${pid}/fd/${fd}`);
      } catch {
        // Closed meanwhile.
      }
      if (/\/\.codex\/sessions\/.*rollout-.*\.jsonl$/.test(f)) return f;
    }
  } catch {
    // Gone, or not this user's.
  }
  return null;
}

/**
 * A file's lines from the end, newest first, reading at most `budget` bytes back; a line longer than `longest` (a run's
 * log may carry whole pictures as base64 on one line) is skipped without being decoded.
 */
export function* backLines(file: string, budget = 64 << 20, longest = 256 << 10): Generator<string> {
  const fd = openSync(file, "r");
  try {
    let pos = fstatSync(fd).size,
      read = 0,
      cur: Buffer[] | null = [],
      len = 0;
    while (pos > 0 && read < budget) {
      const n = Math.min(1 << 20, pos),
        buf = Buffer.alloc(n);
      pos -= n;
      read += n;
      readSync(fd, buf, 0, n, pos);
      for (let i = n; ;) {
        const j = i > 0 ? buf.lastIndexOf(10, i - 1) : -1,
          piece = buf.subarray(j + 1, i);
        if (cur) {
          len += piece.length;
          if (len > longest) cur = null;
          else cur.unshift(piece);
        }
        if (j === -1) break; // the line goes on in the chunk before
        if (cur && len) yield Buffer.concat(cur).toString("utf8");
        cur = [];
        len = 0;
        i = j;
      }
    }
    if (pos === 0 && cur && len) yield Buffer.concat(cur).toString("utf8");
  } finally {
    closeSync(fd);
  }
}
const short = (s: string, n = 160) => {
  const t = s.replace(/\s+/g, " ").trim();
  return t.length > n ? `${t.slice(0, n - 1)}…` : t;
};
type Rollout = {
  timestamp?: string;
  type?: string;
  payload?: { type?: string; role?: string; message?: string; content?: { text?: string }[]; item?: { type?: string; command?: string[] } };
};
const parseLine = (l: string): Rollout | null => {
  try {
    return JSON.parse(l) as Rollout;
  } catch {
    return null;
  }
};

/** What Codex last said in its run (its rollout file): its last message, else what its last command did. */
export function codexLast(file: string): Last | null {
  try {
    let cmd: Last | null = null;
    for (const l of backLines(file)) {
      const e = parseLine(l),
        p = e?.payload;
      if (!e || !p) continue;
      const at = e.timestamp ?? null;
      if (e.type === "response_item" && p.type === "message" && p.role === "assistant") {
        const t = (p.content ?? []).map((c) => c.text ?? "").join(" ");
        if (t.trim()) return { text: short(t), at, from: "codex" };
      }
      if (e.type === "event_msg" && p.type === "agent_message" && p.message) return { text: short(p.message), at, from: "codex" };
      const last = p.item?.command?.at(-1);
      if (!cmd && e.type === "event_msg" && p.item?.type === "CommandExecution" && last)
        cmd = { text: short(last.replaceAll(`${homedir()}/`, "~/")), at, from: "codex" };
    }
    return cmd;
  } catch {
    return null;
  }
}

/**
 * The newest file under a folder (logs aside), in a bounded look: the most recently changed folders first, 5 deep,
 * 20000 entries at most, no dot folders or dependencies. Never the whole home folder or the root.
 */
export function newestFile(dir: string | null | undefined, budget = 20_000): Last | null {
  if (!dir || dir === "/" || dir === homedir() || !existsSync(dir)) return null;
  let best: { path: string; at: number } | null = null,
    seen = 0;
  const queue: { d: string; depth: number; at: number }[] = [{ d: dir, depth: 0, at: Infinity }];
  while (queue.length && seen < budget) {
    queue.sort((x, y) => y.at - x.at);
    const { d, depth } = queue.shift()!;
    let names: Dirent[];
    try {
      names = readdirSync(d, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const n of names) {
      if (++seen > budget) break;
      if (n.name.startsWith(".") || n.name === "node_modules" || n.name === "__pycache__") continue;
      const p = join(d, n.name);
      let m: number;
      try {
        m = statSync(p).mtimeMs;
      } catch {
        continue;
      }
      if (n.isDirectory()) {
        if (depth < 5) queue.push({ d: p, depth: depth + 1, at: m });
      } else if (n.isFile() && !/\.(log|jsonl)$/.test(n.name) && (!best || m > best.at)) best = { path: p, at: m };
    }
  }
  return best ? { text: best.path.slice(dir.length + 1), at: new Date(best.at).toISOString(), from: "file" } : null;
}

/** macOS launchd jobs whose label matches `labels` (none: no jobs listed): running (a pid) or failed (a last exit that isn't 0). */
export async function launchdJobs(labels: string | null): Promise<{ label: string; pid: number | null; status: number }[]> {
  if (!mac || !labels) return [];
  const re = new RegExp(labels, "i");
  const { stdout } = await sh("launchctl", ["list"]).catch((e: { stdout?: string }) => ({ stdout: e.stdout ?? "" }));
  return stdout
    .split("\n")
    .slice(1)
    .map((l) => l.split("\t"))
    .filter((c): c is [string, string, string] => c.length === 3)
    .map(([pid, status, label]) => ({ label, pid: pid === "-" ? null : +pid, status: +status || 0 }))
    .filter((j) => re.test(j.label) && (j.pid || j.status));
}

/** `GET /jobs`: the runs with their last activity, and the matching launchd jobs. */
export async function scan(labels: string | null): Promise<Record<string, unknown>> {
  const list = await runs(await procs());
  return {
    at: new Date().toISOString(),
    procs: list.map((p) => ({ ...p, last: p.rollout ? codexLast(p.rollout) : null, file: newestFile(p.cwd) })),
    launchd: await launchdJobs(labels),
  };
}
