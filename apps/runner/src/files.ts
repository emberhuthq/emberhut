// Files between the server and a session: the server pushes files into a session's inbox (a design, a spreadsheet)
// and may pull a session's results. Only a session's own folders (where it was asked to start, its worktree, its
// inbox) and the temporary folder; never keys, logins or the runner's own data; links are resolved first, `..` is
// refused, and the size is capped both ways. Every transfer and every refusal is kept and logged.
import { createReadStream, createWriteStream, existsSync, mkdirSync, realpathSync, rmSync, statSync } from "node:fs";
import type { IncomingMessage, ServerResponse } from "node:http";
import { homedir, tmpdir } from "node:os";
import { basename, join, sep } from "node:path";
import { Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import { log } from "./log.ts";
import { Refused } from "./sessions/folders.ts";
import type { SessionRow, Store } from "./store.ts";
import { now } from "./store.ts";
import { clip } from "./text.ts";

/** Folders that never leave the machine, even inside a project. */
const SECRET_DIRS = /(^|[/\\])(\.ssh|\.gnupg|\.aws|\.kube|\.docker|\.password-store|Keychains)([/\\]|$)/;
/** Files that never leave the machine. */
const SECRET_FILES =
  /(^|[/\\])(\.env(\.[\w.-]*)?|\.netrc|\.npmrc|\.pypirc|\.credentials\.json|id_(rsa|ed25519|ecdsa|dsa)(\.pub)?|[^/\\]+\.(pem|p12|pfx|key|mobileprovision))$/i;

/** A size in words. */
export const mb = (n: number): string =>
  n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`;
const real = (p: string | null | undefined) => {
  try {
    return p ? realpathSync(p) : null;
  } catch {
    return null;
  }
};

/** The file rules of one runner. */
export class Files {
  /** The temporary folder files may come from. */
  readonly tmp: string;
  private readonly store: Store;
  private readonly home: string;
  private readonly machine: string;
  private readonly max: number;

  constructor(store: Store, home: string, machine: string, max: number, tmp = process.platform === "win32" ? tmpdir() : "/tmp") {
    this.store = store;
    this.home = home;
    this.machine = machine;
    this.max = max;
    this.tmp = tmp;
  }

  /** A session's inbox. */
  inboxOf(id: string): string {
    return join(this.home, "inbox", id);
  }

  private secret(r: string): boolean {
    const own = real(this.home),
      inbox = real(join(this.home, "inbox"));
    return SECRET_DIRS.test(r) || SECRET_FILES.test(r) || (!!own && r.startsWith(own + sep) && !(inbox && r.startsWith(inbox + sep)));
  }

  /** The folders a path may be in: the temporary folder and the session's (or every session's), never / or the home folder. */
  private roots(s?: SessionRow): string[] {
    const rows = s ? [s] : this.store.all<SessionRow>("select id, cwd, origin, worktree from sessions");
    const list = [this.tmp, ...rows.flatMap((x) => [x.cwd, x.origin, x.worktree, this.inboxOf(x.id)])].map(real);
    return [...new Set(list)].filter((r): r is string => !!r && r !== "/" && r !== real(homedir()) && !/^[A-Za-z]:\\?$/.test(r));
  }

  /** Checks a path that may leave the machine: absolute, no `..`, a file in the allowed folders, not private, not too big. */
  check(raw: unknown, s?: SessionRow): { path: string; size: number; mtime: string } {
    const p = (typeof raw === "string" ? raw : "").trim().replace(/^~(?=[/\\])/, homedir());
    if (!p.startsWith("/") && !/^[A-Za-z]:[/\\]/.test(p)) throw new Refused(`not an absolute path: ${clip(p, 200)}`);
    if (p.split(/[/\\]/).includes("..")) throw new Refused(`no .. in paths: ${clip(p, 200)}`);
    const r = real(p);
    if (!r) throw new Refused(`no file at ${p} on ${this.machine}`, 404);
    if (!this.roots(s).some((x) => r === x || r.startsWith(x + sep)))
      throw new Refused(`${p} is outside ${s ? `session ${s.id}'s folders` : "the sessions' folders"} and ${this.tmp} on ${this.machine}`);
    if (this.secret(r)) throw new Refused(`${p} is private`);
    const st = statSync(r);
    if (!st.isFile()) throw new Refused(`${p} is not a file`);
    if (st.size > this.max) throw new Refused(`${p} is ${mb(st.size)}, over the ${mb(this.max)} limit`, 413);
    return { path: r, size: st.size, mtime: st.mtime.toISOString() };
  }

  private record(
    dir: "out" | "in",
    session: string | null,
    path: string,
    size: number | null,
    by: string,
    ok: boolean,
    why?: string,
  ): void {
    this.store.run(
      "insert into transfers(at, dir, session, path, size, by, ok, why) values (?, ?, ?, ?, ?, ?, ?, ?)",
      now(),
      dir,
      session,
      path,
      size,
      by,
      ok ? 1 : 0,
      why ?? null,
    );
    log(
      `file ${dir}${ok ? "" : " refused"}: ${path}${size != null ? ` (${mb(size)})` : ""}${session ? ` session ${session}` : ""} by ${by}${why ? `: ${why}` : ""}`,
    );
  }

  /** `GET /files?path=&session=&by=` (`stat=1`: only what it is): the file as it is now. */
  serve(url: URL, res: ServerResponse, send: (res: ServerResponse, code: number, body: unknown) => void): void {
    const raw = url.searchParams.get("path") ?? "",
      sid = url.searchParams.get("session"),
      by = clip(url.searchParams.get("by") || "server", 40);
    const s = sid ? this.store.session(sid) : undefined;
    let f: ReturnType<Files["check"]>;
    try {
      if (sid && !s) throw new Refused(`no session ${sid} on ${this.machine}`, 404);
      f = this.check(raw, s);
    } catch (e) {
      const why = e instanceof Error ? e.message : String(e);
      this.record("out", sid, clip(raw, 400), null, by, false, why);
      return send(res, e instanceof Refused ? e.code : 400, { error: why });
    }
    if (url.searchParams.get("stat")) return send(res, 200, { ...f, machine: this.machine });
    res.writeHead(200, {
      "Content-Type": "application/octet-stream",
      "Content-Length": f.size,
      "X-Path": encodeURIComponent(f.path),
      "X-Mtime": f.mtime,
    });
    let done = false;
    res.on("finish", () => {
      done = true;
      this.record("out", sid, f.path, f.size, by, true);
    });
    res.on("close", () => {
      if (!done) this.record("out", sid, f.path, f.size, by, false, "cut off before the end");
    });
    createReadStream(f.path)
      .on("error", () => res.destroy())
      .pipe(res);
  }

  /** `PUT /sessions/{id}/files?name=&by=&size=` (the raw file as the body): into the session's inbox, under a free name. */
  async receive(s: SessionRow, url: URL, req: IncomingMessage): Promise<{ path: string; size: number; inbox: string; machine: string }> {
    const by = clip(url.searchParams.get("by") || "server", 40);
    const name =
      basename(url.searchParams.get("name") ?? "")
        .replace(/[^\w.\- ]+/g, "_")
        .replace(/^[.\s]+/, "")
        .slice(-120) || "file";
    const said = Number(req.headers["content-length"] ?? url.searchParams.get("size") ?? 0);
    const dir = this.inboxOf(s.id);
    if (said > this.max) {
      req.resume();
      this.record("in", s.id, join(dir, name), said, by, false, `over the ${mb(this.max)} limit`);
      throw new Refused(`${name} is ${mb(said)}, over the ${mb(this.max)} limit`, 413);
    }
    mkdirSync(dir, { recursive: true });
    const dot = name.lastIndexOf("."),
      stem = dot > 0 ? name.slice(0, dot) : name,
      ext = dot > 0 ? name.slice(dot) : "";
    let path = join(dir, name);
    for (let i = 2; existsSync(path); i++) path = join(dir, `${stem}-${i}${ext}`);
    let n = 0;
    const max = this.max;
    const cap = new Transform({
      transform(c: Buffer, _e, cb) {
        n += c.length;
        cb(n > max ? new Refused(`${name} is over the ${mb(max)} limit`, 413) : null, c);
      },
    });
    try {
      await pipeline(req, cap, createWriteStream(path, { flags: "wx" }));
    } catch (e) {
      rmSync(path, { force: true });
      const why = e instanceof Error ? e.message : String(e);
      this.record("in", s.id, path, n, by, false, why);
      throw e instanceof Refused ? e : new Refused(`the file didn't arrive whole: ${why}`, 400);
    }
    this.record("in", s.id, path, n, by, true);
    return { path, size: n, inbox: dir, machine: this.machine };
  }

  /** Files that arrived for a session since its last turn, marked told. */
  arrived(id: string): { path: string; size: number; by: string }[] {
    const rows = this.store.all<{ id: number; path: string; size: number; by: string }>(
      "select id, path, size, by from transfers where session = ? and dir = 'in' and ok = 1 and told = 0 order by id",
      id,
    );
    for (const f of rows) this.store.run("update transfers set told = 1 where id = ?", f.id);
    return rows;
  }
}
