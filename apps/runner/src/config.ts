// The runner's settings: where its files live, its name on the server, the server's address, its token, the port it
// listens on, the worktrees, the account groups and the instructions its sessions get. All of it comes from one JSON
// file in the runner's home folder, with a few environment variables on top for service managers and tests; nothing
// about one installation is written in the code.
import { existsSync, readFileSync } from "node:fs";
import { homedir, hostname, platform } from "node:os";
import { join, resolve } from "node:path";

/** Work that belongs to a group (a client, a team): a session is in it when its project or its folder matches. */
export type Group = {
  /** A case-insensitive regular expression over the session's project name. */
  projects?: string;
  /** A case-insensitive regular expression over the folder the session was asked to start in. */
  folders?: string;
};

/** Text added to every session's system prompt, after the agent's own. */
export type Instructions = {
  /** For sessions the assistant opens for work. */
  work?: string;
  /** For the owner's own sessions, opened and driven from an app. */
  direct?: string;
  /** About this machine (what it has, what to leave alone), for both. */
  machine?: string;
};

/** The runner's settings, resolved: every path absolute, every default filled in. */
export type Config = {
  /** The runner's home: its database, accounts file, inboxes and secrets. */
  home: string;
  /** The runner's name on the server (`[A-Za-z0-9_-]`); the server knows its runners by it. */
  name: string;
  /** The server's address (`https://…`, or `http://` on loopback or a private network), or null before pairing. */
  server: string | null;
  /** The file holding the runner's token: the server presents it to the runner, and the runner to the server. */
  tokenFile: string;
  /** The port the runner's API listens on. */
  port: number;
  /** The addresses it listens on. */
  listen: string[];
  /** The address the server calls it at, when pairing (null: `http://<first listen address>:<port>`). */
  advertise: string | null;
  /** The accounts file. */
  accountsFile: string;
  /** Where session worktrees are made: `<worktreesDir>/<repo>/<branch>`. */
  worktreesDir: string;
  /** The prefix of the branch a session's worktree gets when it names none. */
  branchPrefix: string;
  /** The model sessions run on when they name none (null: the agent's own default). */
  model: string | null;
  /** The account that goes first for any work it may take (null: the list's order). */
  defaultAccount: string | null;
  /** Work groups by name. */
  groups: Record<string, Group>;
  /** Text added to the sessions' system prompts. */
  instructions: Instructions;
  /** A folder of skills linked into every account's skills folder (null: none). */
  skillsDir: string | null;
  /** How often a machine sample goes to the server, in ms. */
  machineEvery: number;
  /** The largest file taken in or sent out, in bytes. */
  maxFileBytes: number;
  /** Whether sessions get the MCP servers the server hands out. */
  shareMcp: boolean;
  /** macOS: a regular expression over launchd labels whose jobs the Background jobs page lists (null: none). */
  jobLabels: string | null;
};

/** The settings file's name in the runner's home. */
export const CONFIG_FILE = "config.json";
/** The runner's API port when none is set. */
export const DEFAULT_PORT = 8806;

/** The runner's home when `EMBERHUT_RUNNER_HOME` doesn't say: the platform's usual place for an app's own data. */
export function defaultHome(env: NodeJS.ProcessEnv = process.env, os: NodeJS.Platform = platform()): string {
  const h = homedir();
  if (os === "darwin") return join(h, "Library", "Application Support", "Emberhut Runner");
  if (os === "win32") return join(env.LOCALAPPDATA ?? join(h, "AppData", "Local"), "Emberhut Runner");
  return join(env.XDG_DATA_HOME ?? join(h, ".local", "share"), "emberhut-runner");
}

/** `~` or `~/…` as the home folder. */
export const untilde = (p: string): string => (p === "~" || p.startsWith("~/") ? join(homedir(), p.slice(1)) : p);

/** A runner name the server takes: letters, digits, `-` and `_`, 1 to 40 of them. */
export const NAME = /^[A-Za-z0-9_-]{1,40}$/;

/** This machine's name as a runner name. */
export const machineName = (): string =>
  hostname()
    .split(".")[0]!
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40) || "runner";

/** A setting that is wrong, with the file and key it came from. */
export class ConfigError extends Error {}

const str = (v: unknown, key: string): string | undefined => {
  if (v === undefined || v === null) return undefined;
  if (typeof v !== "string") throw new ConfigError(`${key} must be text`);
  return v;
};
const num = (v: unknown, key: string): number | undefined => {
  if (v === undefined || v === null || v === "") return undefined;
  const n = typeof v === "number" ? v : Number(v);
  if (!Number.isFinite(n) || n <= 0) throw new ConfigError(`${key} must be a positive number`);
  return n;
};

/** A port: 1-65535, or 0 for one the system picks. */
const port = (v: unknown, key: string): number | undefined => {
  if (v === undefined || v === null || v === "") return undefined;
  const n = typeof v === "number" ? v : Number(v);
  if (!Number.isInteger(n) || n < 0 || n > 65535) throw new ConfigError(`${key} must be a port (0-65535)`);
  return n;
};

/** The settings file of `home` as written, or `{}` when there is none. */
export function readConfigFile(home: string): Record<string, unknown> {
  const f = join(home, CONFIG_FILE);
  if (!existsSync(f)) return {};
  let raw: unknown;
  try {
    raw = JSON.parse(readFileSync(f, "utf8"));
  } catch (e) {
    throw new ConfigError(`${f} is not JSON: ${e instanceof Error ? e.message : String(e)}`);
  }
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) throw new ConfigError(`${f} must hold a JSON object`);
  return raw as Record<string, unknown>;
}

/**
 * The runner's settings: `<home>/config.json`, then the environment (`EMBERHUT_RUNNER_NAME`, `EMBERHUT_SERVER`,
 * `EMBERHUT_RUNNER_PORT`, `EMBERHUT_RUNNER_TOKEN_FILE`, `EMBERHUT_RUNNER_ACCOUNTS`), then the defaults.
 */
export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const home = resolve(untilde(env.EMBERHUT_RUNNER_HOME ?? defaultHome(env)));
  const f = readConfigFile(home);
  const path = (v: string | undefined, fallback: string) => resolve(home, untilde(v ?? fallback));

  const name = env.EMBERHUT_RUNNER_NAME ?? str(f.name, "name") ?? machineName();
  if (!NAME.test(name)) throw new ConfigError(`name ${JSON.stringify(name)}: letters, digits, - and _ only (40 at most)`);
  const server = (env.EMBERHUT_SERVER ?? str(f.server, "server") ?? null)?.replace(/\/+$/, "") ?? null;
  if (server && !/^https?:\/\/[^/\s]+/.test(server))
    throw new ConfigError(`server ${JSON.stringify(server)} must start with http:// or https://`);
  const listen = f.listen ?? ["127.0.0.1"];
  if (!Array.isArray(listen) || !listen.length || !listen.every((x) => typeof x === "string" && x))
    throw new ConfigError("listen must be a list of addresses");
  const advertise = str(f.advertise, "advertise")?.replace(/\/+$/, "") ?? null;
  if (advertise && !/^https?:\/\//.test(advertise)) throw new ConfigError("advertise must start with http:// or https://");

  const groups: Record<string, Group> = {};
  const g = f.groups ?? {};
  if (typeof g !== "object" || Array.isArray(g)) throw new ConfigError("groups must be an object of name: { projects, folders }");
  for (const [k, v] of Object.entries(g as Record<string, unknown>)) {
    if (!v || typeof v !== "object") throw new ConfigError(`groups.${k} must be an object`);
    const o = v as Record<string, unknown>;
    const projects = str(o.projects, `groups.${k}.projects`),
      folders = str(o.folders, `groups.${k}.folders`);
    for (const [key, re] of [
      ["projects", projects],
      ["folders", folders],
    ] as const)
      if (re)
        try {
          new RegExp(re, "i");
        } catch {
          throw new ConfigError(`groups.${k}.${key} is not a regular expression`);
        }
    groups[k] = { ...(projects ? { projects } : {}), ...(folders ? { folders } : {}) };
  }
  const ins = (f.instructions ?? {}) as Record<string, unknown>;
  const instructions: Instructions = {};
  for (const k of ["work", "direct", "machine"] as const) {
    const v = str(ins[k], `instructions.${k}`);
    if (v) instructions[k] = v;
  }
  const skills = str(f.skillsDir, "skillsDir");

  return {
    home,
    name,
    server,
    tokenFile: path(env.EMBERHUT_RUNNER_TOKEN_FILE ?? str(f.tokenFile, "tokenFile"), "token"),
    port: port(env.EMBERHUT_RUNNER_PORT, "EMBERHUT_RUNNER_PORT") ?? port(f.port, "port") ?? DEFAULT_PORT,
    listen: listen as string[],
    advertise,
    accountsFile: path(env.EMBERHUT_RUNNER_ACCOUNTS ?? str(f.accountsFile, "accountsFile"), "accounts.json"),
    worktreesDir: resolve(untilde(str(f.worktreesDir, "worktreesDir") ?? join(homedir(), "Documents", "worktrees"))),
    branchPrefix: str(f.branchPrefix, "branchPrefix") ?? "emberhut/",
    model: str(f.model, "model") ?? null,
    defaultAccount: str(f.defaultAccount, "defaultAccount") ?? null,
    groups,
    instructions,
    skillsDir: skills ? resolve(home, untilde(skills)) : null,
    machineEvery: num(f.machineEvery, "machineEvery") ?? 20_000,
    maxFileBytes: num(f.maxFileBytes, "maxFileBytes") ?? 200 * 1024 * 1024,
    shareMcp: f.shareMcp !== false,
    jobLabels: str(f.jobLabels, "jobLabels") ?? null,
  };
}

/** The runner's token, or null when its file is missing or empty. */
export function readToken(c: Pick<Config, "tokenFile">): string | null {
  try {
    return readFileSync(c.tokenFile, "utf8").trim() || null;
  } catch {
    return null;
  }
}

/** The address the server calls this runner at. */
export const advertised = (c: Pick<Config, "advertise" | "listen" | "port">): string =>
  c.advertise ?? `http://${c.listen[0]!.includes(":") ? `[${c.listen[0]!}]` : c.listen[0]!}:${c.port}`;

/** The runner's secrets folder (a provider's key and region, for instance), inside its home. */
export const secretsDir = (c: Pick<Config, "home">): string => join(c.home, "secrets");
