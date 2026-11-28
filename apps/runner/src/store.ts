// The runner's own database (SQLite, node:sqlite): its sessions, their turns, every step of their transcripts, the
// events the server hasn't taken yet and every file in or out. One file in the runner's home; a restart reads it back
// and carries on what was running.
import { DatabaseSync, type SQLInputValue } from "node:sqlite";
import { join } from "node:path";

/** A row as SQLite gives it. */
export type Row = Record<string, SQLInputValue>;

/** A session as stored. */
export type SessionRow = {
  id: string;
  title: string | null;
  cwd: string;
  origin: string | null;
  project: string | null;
  model: string | null;
  effort: string | null;
  account: string | null;
  claude: string | null;
  materialized: number;
  status: string;
  worktree: string | null;
  branch: string | null;
  meta: string | null;
  seq: number;
  resume_at: string | null;
  background: number;
  created: string | null;
  updated: string | null;
};

/** A turn as stored: one message and its answer. */
export type TurnRow = {
  id: string;
  session: string;
  n: number;
  prompt: string | null;
  by: string | null;
  requested: string | null;
  started: string | null;
  ended: string | null;
  outcome: string | null;
  last: string | null;
  accounts: string | null;
};

/** A transcript step as stored; `data` is the step's own fields as JSON. */
export type EventRow = { seq: number; session: string; turn: string | null; at: string; kind: string; data: string };

/** The columns a session's update may set. */
export type SessionUpdate = Partial<Omit<SessionRow, "id" | "updated" | "created">>;

const SESSION_COLUMNS = new Set<string>([
  "title",
  "cwd",
  "origin",
  "project",
  "model",
  "effort",
  "account",
  "claude",
  "materialized",
  "status",
  "worktree",
  "branch",
  "meta",
  "seq",
  "resume_at",
  "background",
]);

/** Now, as the store writes times. */
export const now = (): string => new Date().toISOString();

/** The runner's database. */
export class Store {
  readonly db: DatabaseSync;

  /** Opens (or makes) `runner.db` in `home`. */
  constructor(home: string, file = "runner.db") {
    this.db = new DatabaseSync(file === ":memory:" ? file : join(home, file));
    // WAL with synchronous=normal: a commit survives the process crashing and is only at risk on power loss, without an
    // fsync per step (a busy disk made each one take a tenth of a second).
    this.db.exec(`pragma journal_mode = wal; pragma synchronous = normal; pragma busy_timeout = 10000;
      create table if not exists sessions (id text primary key, title text, cwd text not null, project text, model text, effort text,
        account text, claude text, materialized integer not null default 0, status text not null default 'queued', worktree text, branch text,
        meta text, seq integer not null default 0, resume_at text, created text, updated text);
      create table if not exists turns (id text primary key, session text not null, n integer not null, prompt text, by text, requested text,
        started text, ended text, outcome text, last text, accounts text);
      create table if not exists events (seq integer primary key autoincrement, session text not null, turn text, at text, kind text, data text);
      create index if not exists events_session on events(session, seq);
      create table if not exists outbox (id integer primary key autoincrement, session text, kind text, body text);
      create table if not exists transfers (id integer primary key autoincrement, at text, dir text, session text, path text, size integer,
        by text, ok integer, why text, told integer not null default 0);`);
    // Columns added after the first schema; a database made before them gets them here.
    for (const col of ["background integer not null default 0", "origin text"])
      try {
        this.db.exec(`alter table sessions add column ${col}`);
      } catch {
        // There already.
      }
  }

  /** One row, or undefined. */
  one<T = Row>(sql: string, ...a: SQLInputValue[]): T | undefined {
    return this.db.prepare(sql).get(...a) as T | undefined;
  }
  /** Every row. */
  all<T = Row>(sql: string, ...a: SQLInputValue[]): T[] {
    return this.db.prepare(sql).all(...a) as T[];
  }
  /** Runs a statement; returns the new row's id. */
  run(sql: string, ...a: SQLInputValue[]): number {
    return Number(this.db.prepare(sql).run(...a).lastInsertRowid);
  }

  /** A session. */
  session(id: string): SessionRow | undefined {
    return this.one<SessionRow>("select * from sessions where id = ?", id);
  }
  /** A turn. */
  turn(id: string): TurnRow | undefined {
    return this.one<TurnRow>("select * from turns where id = ?", id);
  }
  /** Changes a session's columns (only known ones) and its `updated` time. */
  setSession(id: string, v: SessionUpdate): void {
    for (const [k, x] of Object.entries(v)) {
      if (!SESSION_COLUMNS.has(k)) throw new Error(`no session column ${k}`);
      this.run(`update sessions set ${k} = ? where id = ?`, x ?? null, id);
    }
    this.run("update sessions set updated = ? where id = ?", now(), id);
  }
  /** The next turn of a session that hasn't started, oldest first. */
  nextTurn(session: string): TurnRow | undefined {
    return this.one<TurnRow>("select * from turns where session = ? and started is null and ended is null order by n limit 1", session);
  }
  /** How many turns of a session wait behind the running one. */
  queued(session: string): number {
    return Number(
      this.one<{ n: number }>("select count(*) as n from turns where session = ? and started is null and ended is null", session)?.n ?? 0,
    );
  }
  /** Writes a transcript step; returns its sequence number. */
  addEvent(session: string, turn: string | null, kind: string, data: object): number {
    return this.run(
      "insert into events(session, turn, at, kind, data) values (?, ?, ?, ?, ?)",
      session,
      turn,
      now(),
      kind,
      JSON.stringify(data),
    );
  }
  /** A step by its sequence number. */
  event(seq: number): EventRow | undefined {
    return this.one<EventRow>("select * from events where seq = ?", seq);
  }

  /** Closes the database. */
  close(): void {
    this.db.close();
  }
}
