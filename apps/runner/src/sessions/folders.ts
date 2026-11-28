// Folders on this machine for the owner's new-session sheet (where sessions started lately, the folders inside one),
// a repository's branches, and the git worktree a session may get of its own.
import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync, readdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join, resolve, sep } from "node:path";
import type { Store } from "../store.ts";

const HOME = homedir();
const git = (cwd: string, ...a: string[]) =>
  execFileSync("git", ["-C", cwd, ...a], { encoding: "utf8", timeout: 60_000, stdio: ["ignore", "pipe", "pipe"] }).trim();
const isRepo = (d: string) => existsSync(join(d, ".git"));
const untildePath = (p: string) => resolve(p.replace(/^~(?=\/|$)/, HOME));
const inHome = (d: string) => d === HOME || d.startsWith(HOME + sep);

/** A refusal the API answers with its status and the words. */
export class Refused extends Error {
  /** The HTTP status the API answers with. */
  readonly code: number;

  constructor(message: string, code = 403) {
    super(message);
    this.code = code;
  }
}

/** A folder in a listing. */
export type Folder = { path: string; name: string; git: boolean; used?: string };

/** The start of the sheet: where sessions here started lately (12, newest first), and the documents folder. */
export function startFolders(store: Store, machine: string): { machine: string; home: string; recent: Folder[]; documents: string } {
  const seen = new Set<string>(),
    recent: Folder[] = [];
  for (const s of store.all<{ origin: string | null; cwd: string; updated: string | null }>(
    "select origin, cwd, updated from sessions order by updated desc limit 200",
  )) {
    const d = s.origin ?? s.cwd;
    if (seen.has(d) || !existsSync(d)) continue;
    seen.add(d);
    recent.push({ path: d, name: basename(d), git: isRepo(d), ...(s.updated ? { used: s.updated } : {}) });
    if (recent.length >= 12) break;
  }
  const documents = join(HOME, "Documents");
  return { machine, home: HOME, recent, documents: existsSync(documents) ? documents : HOME };
}

/** The folders inside one (inside the home folder only; no hidden ones, no dependencies folders), 300 at most. */
export function foldersIn(path: string, machine: string): { machine: string; home: string; path: string; git: boolean; folders: Folder[] } {
  const dir = untildePath(path);
  if (!inHome(dir) || !existsSync(dir) || !statSync(dir).isDirectory()) throw new Refused(`no folder ${path} on ${machine}`, 404);
  const folders = readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isDirectory() && !e.name.startsWith(".") && e.name !== "node_modules")
    .map((e) => ({ path: join(dir, e.name), name: e.name, git: isRepo(join(dir, e.name)) }))
    .sort((a, b) => a.name.localeCompare(b.name))
    .slice(0, 300);
  return { machine, home: HOME, path: dir, git: isRepo(dir), folders };
}

/** A repository's branches, local and remote, newest first (100 at most), and the one it is on. */
export function branches(cwd: string): { repo: string; current: string; branches: string[] } {
  let top: string;
  try {
    top = git(untildePath(cwd || "."), "rev-parse", "--show-toplevel");
  } catch {
    throw new Refused("not a git folder", 400);
  }
  const list = git(top, "for-each-ref", "--sort=-committerdate", "--format=%(refname:short)", "refs/heads", "refs/remotes/origin")
    .split("\n")
    .filter((b) => b && b !== "origin/HEAD" && b !== "origin")
    .map((b) => b.replace(/^origin\//, ""));
  return { repo: top, current: git(top, "rev-parse", "--abbrev-ref", "HEAD"), branches: [...new Set(list)].slice(0, 100) };
}

/** What a session asked for as its worktree: `true`, or a branch and the commit to start it from. */
export type WorktreeAsk = true | { repo?: string; branch?: string; base?: string };

const slug = (s: string) =>
  s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 40) || "session";

/**
 * A git worktree for a session, under `<root>/<repo>/<branch>`: on a new branch (`<prefix><title>-<4 hex>` when none is
 * named), on a local branch that exists, or tracking the remote's.
 */
export function makeWorktree(
  cwd: string,
  ask: WorktreeAsk,
  title: string,
  root: string,
  prefix: string,
): { dir: string; branch: string; top: string } {
  const w = ask === true ? {} : ask;
  let top: string;
  try {
    top = git(resolve(cwd, w.repo ?? "."), "rev-parse", "--show-toplevel");
  } catch {
    throw new Refused(`${cwd} is not in a git repository, so it can't have a worktree`, 400);
  }
  const branch = w.branch?.trim() || `${prefix}${slug(title)}-${randomBytes(2).toString("hex")}`;
  if (!/^[\w./-]+$/.test(branch) || branch.includes("..") || branch.startsWith("-")) throw new Refused(`not a branch name: ${branch}`, 400);
  const has = (ref: string) => {
    try {
      git(top, "rev-parse", "--verify", "--quiet", ref);
      return true;
    } catch {
      return false;
    }
  };
  const leaf = (prefix && branch.startsWith(prefix) ? branch.slice(prefix.length) : branch).replace(/[^\w.-]+/g, "-");
  let dir = join(root, basename(top), leaf);
  for (let i = 2; existsSync(dir); i++) dir = `${dir.replace(/-\d+$/, "")}-${i}`;
  if (has(`refs/heads/${branch}`)) git(top, "worktree", "add", dir, branch);
  else if (!w.base && has(`refs/remotes/origin/${branch}`)) git(top, "worktree", "add", "--track", "-b", branch, dir, `origin/${branch}`);
  else git(top, "worktree", "add", dir, "-b", branch, w.base ?? "HEAD");
  return { dir, branch, top };
}

/** The instruction files the agent loads for a folder: its own and those above it, inside the home folder. */
export function instructionFiles(dir: string): string[] {
  const out: string[] = [];
  for (let d = resolve(dir); d.startsWith(HOME + sep); d = resolve(d, ".."))
    for (const f of [join(d, "CLAUDE.md"), join(d, ".claude", "CLAUDE.md")]) if (existsSync(f)) out.unshift(f);
  return out;
}
