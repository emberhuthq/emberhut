// Where each account's agent keeps its login, and what a session on it runs with: its configuration folder (shared
// parts linked to the default folder, so a session moves between accounts with its transcript), its environment, and
// the one place a project's memory lives whichever account runs it. No token is read here: only whether one is there.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, realpathSync, symlinkSync, unlinkSync } from "node:fs";
import { homedir, platform } from "node:os";
import { dirname, join, resolve } from "node:path";
import { untilde } from "../config.ts";
import { providerOf, type Account } from "./list.ts";

/** The agent's default configuration folder (its own login when no folder is set). */
export const DEFAULT_DIR = join(homedir(), ".claude");

/** An account's configuration folder. */
export const dirOf = (a: Account): string => (a.configDir ? resolve(untilde(a.configDir)) : DEFAULT_DIR);

/** What a fresh account folder shares with the default one, so sessions, settings and skills follow it. */
const SHARED = ["projects", "settings.json", "CLAUDE.md", "agents", "commands", "skills", "plugins"];

/** A secret of the runner's (a provider's key), or null. */
export function secret(secrets: string, file: string | undefined): string | null {
  if (!file || file.includes("/") || file.includes("\\") || file.startsWith(".")) return null;
  try {
    return readFileSync(join(secrets, file), "utf8").trim() || null;
  } catch {
    return null;
  }
}

/** The key file an account reads: its own, or the provider's usual name. */
export const keyFileOf = (a: Account): string | undefined =>
  a.keyFile ?? (providerOf(a) === "bedrock" ? "aws-bedrock-api-key" : providerOf(a) === "api" ? "anthropic-api-key" : undefined);

/** Whether an account can run now: a login in its folder (on macOS, in the login Keychain), or its key. */
export class Logins {
  private readonly seen = new Map<string, { ok: boolean; at: number }>();
  private readonly secrets: string;
  private readonly files: boolean;

  /** `files`: look only at credential files, never the Keychain (tests, and machines without one). */
  constructor(secrets: string, files = platform() !== "darwin") {
    this.secrets = secrets;
    this.files = files;
  }

  /** Whether the account has its login or key; on macOS asked of the Keychain at most once a minute (only that it exists). */
  loggedIn(a: Account): boolean {
    if (providerOf(a) !== "claude") return !!secret(this.secrets, keyFileOf(a)) && (providerOf(a) !== "bedrock" || !!a.region);
    if (this.files) return existsSync(join(dirOf(a), ".credentials.json"));
    const hit = this.seen.get(a.id);
    if (hit && Date.now() - hit.at < 60_000) return hit.ok;
    // The agent's Keychain item: "Claude Code-credentials" for the default folder, "-<first 8 hex of sha256(folder)>" for others.
    const service = a.configDir
      ? `Claude Code-credentials-${createHash("sha256").update(dirOf(a)).digest("hex").slice(0, 8)}`
      : "Claude Code-credentials";
    const ok = spawnSync("/usr/bin/security", ["find-generic-password", "-s", service], { stdio: "ignore", timeout: 10_000 }).status === 0;
    this.seen.set(a.id, { ok, at: Date.now() });
    return ok;
  }

  /** Forgets what the Keychain said about an account. */
  forget(id: string): void {
    this.seen.delete(id);
  }

  /** The environment a session on this account runs with: its folder, and for a provider its key and endpoint. */
  envFor(a: Account, skillsDir: string | null, base: NodeJS.ProcessEnv = process.env): Record<string, string> {
    prepareDir(a, skillsDir);
    const env: Record<string, string> = {};
    for (const [k, v] of Object.entries(base)) if (v !== undefined) env[k] = v;
    // Nothing of the runner's own agent, login or provider may leak into a session on another account.
    for (const k of [
      "CLAUDECODE",
      "CLAUDE_CODE_OAUTH_TOKEN",
      "CLAUDE_CODE_USE_BEDROCK",
      "AWS_BEARER_TOKEN_BEDROCK",
      "ANTHROPIC_API_KEY",
      "ANTHROPIC_AUTH_TOKEN",
      "ANTHROPIC_BASE_URL",
      "ANTHROPIC_DEFAULT_OPUS_MODEL",
      "ANTHROPIC_DEFAULT_SONNET_MODEL",
      "ANTHROPIC_DEFAULT_HAIKU_MODEL",
      "ANTHROPIC_SMALL_FAST_MODEL",
    ])
      delete env[k];
    // The default folder must run without CLAUDE_CONFIG_DIR, or the agent reads another file for its settings.
    if (a.configDir) env.CLAUDE_CONFIG_DIR = dirOf(a);
    else delete env.CLAUDE_CONFIG_DIR;
    const key = secret(this.secrets, keyFileOf(a)) ?? "";
    if (providerOf(a) === "api") {
      if (a.baseUrl) Object.assign(env, { ANTHROPIC_BASE_URL: a.baseUrl, ANTHROPIC_AUTH_TOKEN: key });
      else env.ANTHROPIC_API_KEY = key;
    }
    if (providerOf(a) === "bedrock") {
      Object.assign(env, { CLAUDE_CODE_USE_BEDROCK: "1", AWS_BEARER_TOKEN_BEDROCK: key, AWS_REGION: a.region ?? "" });
      // Its sub-agents' aliases (opus, sonnet, haiku) on the provider's ids.
      const m = a.models ?? {},
        pick = (re: RegExp) => Object.entries(m).find(([k]) => re.test(k))?.[1];
      const opus = pick(/^claude-opus/),
        sonnet = pick(/^claude-sonnet/),
        haiku = pick(/^claude-haiku/);
      if (opus) env.ANTHROPIC_DEFAULT_OPUS_MODEL = opus;
      if (sonnet) env.ANTHROPIC_DEFAULT_SONNET_MODEL = sonnet;
      if (haiku) env.ANTHROPIC_DEFAULT_HAIKU_MODEL = haiku;
    }
    return env;
  }
}

/**
 * Makes an account's folder: a new one links the default folder's shared parts (transcripts, settings, instructions,
 * agents, commands, plugins), so any account resumes any session. With a skills folder set, its skills folder is its
 * own (each default skill linked one by one), so linking those skills never writes into the default folder's.
 */
export function prepareDir(a: Account, skillsDir: string | null): string {
  if (!a.configDir) {
    shareSkills(DEFAULT_DIR, skillsDir);
    return DEFAULT_DIR;
  }
  const dir = dirOf(a);
  const fresh = !existsSync(dir);
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  if (fresh || !existsSync(join(dir, "projects")))
    for (const f of SHARED) {
      const from = join(DEFAULT_DIR, f),
        to = join(dir, f);
      if (!existsSync(from) || existsSync(to)) continue;
      if (f === "skills" && skillsDir) {
        mkdirSync(to, { recursive: true });
        for (const k of readdirSync(from)) if (!existsSync(join(to, k))) symlinkSync(join(from, k), join(to, k));
      } else symlinkSync(from, to);
    }
  shareSkills(dir, skillsDir);
  return dir;
}

/** Links each skill of `skillsDir` into the folder's skills, once (a dangling link is made again). */
export function shareSkills(folder: string, skillsDir: string | null): void {
  if (!skillsDir || !existsSync(skillsDir)) return;
  const dir = join(folder, "skills");
  mkdirSync(dir, { recursive: true });
  for (const s of readdirSync(skillsDir)) {
    const to = join(dir, s);
    if (existsSync(to)) continue;
    try {
      if (lstatSync(to).isSymbolicLink()) unlinkSync(to);
    } catch {
      // Not there.
    }
    symlinkSync(join(skillsDir, s), to);
  }
}

/**
 * Puts a session's transcript where this account's agent looks for it: found under any listed account's folder (the
 * real file, not a link) and linked into this one's. False when no account has it yet (a session never started).
 */
export function linkSession(sessionId: string, a: Account, all: Account[]): boolean {
  const dirs = [...new Set(all.map(dirOf).concat(DEFAULT_DIR))].filter((d) => existsSync(join(d, "projects")));
  let found: { slug: string; file: string } | null = null;
  outer: for (const d of dirs)
    for (const slug of readdirSync(join(d, "projects"))) {
      const f = join(d, "projects", slug, `${sessionId}.jsonl`);
      if (existsSync(f) && !lstatSync(f).isSymbolicLink()) {
        found = { slug, file: realpathSync(f) };
        break outer;
      }
    }
  if (!found) return false;
  const mine = join(dirOf(a), "projects", found.slug, `${sessionId}.jsonl`);
  if (existsSync(mine)) return true;
  mkdirSync(dirname(mine), { recursive: true });
  symlinkSync(found.file, mine);
  // Its sub-agents' transcripts, in the folder beside it.
  const sub = found.file.replace(/\.jsonl$/, ""),
    mineSub = mine.replace(/\.jsonl$/, "");
  if (existsSync(sub) && !existsSync(mineSub)) symlinkSync(sub, mineSub);
  return true;
}

/**
 * One memory per project on this machine, whichever account runs a session there: the default folder's, under the
 * project's path (the git repository the folder is in, a worktree's main checkout, else the folder itself).
 */
export function memoryDir(cwd: string): string {
  let root = resolve(cwd);
  const git = spawnSync("git", ["-C", root, "rev-parse", "--path-format=absolute", "--git-common-dir"], {
    encoding: "utf8",
    timeout: 10_000,
  });
  const common = git.status === 0 ? git.stdout.trim() : "";
  if (common.endsWith("/.git")) root = dirname(common);
  try {
    root = realpathSync(root);
  } catch {
    // As given.
  }
  const dir = join(DEFAULT_DIR, "projects", root.replace(/[^A-Za-z0-9]/g, "-"), "memory");
  try {
    return realpathSync(dir);
  } catch {
    return dir;
  }
}
