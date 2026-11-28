// The accounts a runner may run sessions on: one agent configuration folder per subscription login, or a provider
// paid per token (an API key, AWS Bedrock). The list is a JSON file the owner edits (`emberhut-runner accounts add`
// writes it too); the runner never holds a login itself: the agent keeps it in that account's folder.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import type { Group } from "../config.ts";

/** How an account reaches the model. */
export type Provider = "claude" | "api" | "bedrock";

/** One account, as the accounts file lists it. */
export type Account = {
  /** Its id (`[A-Za-z0-9_-]`), how the server and the sessions name it. */
  id: string;
  /** Its name in words. */
  name: string;
  /** `claude` (a subscription login in its folder, the default), `api` (a key, paid per token) or `bedrock`. */
  provider?: Provider;
  /** The agent's configuration folder for this account (null or absent: the agent's default folder). */
  configDir?: string | null;
  /** The login it must have; another login in its folder is never picked. */
  email?: string;
  /** Tried first when lower (none: after every numbered one). */
  order?: number;
  /** False: listed, never picked or looked at. */
  enabled?: boolean;
  /** A line for the owner. */
  note?: string;
  /** The work groups it takes (config.json's `groups`). */
  groups?: string[];
  /** It takes only its groups' work. */
  groupsOnly?: boolean;
  /** Picked on its own only for work that asks for it (by its id or a model only it runs), or when every other is out. */
  optIn?: boolean;
  /** `api` and `bedrock`: the file in the runner's secrets folder holding the key. */
  keyFile?: string;
  /** `api`: an Anthropic-compatible endpoint (none: Anthropic's). */
  baseUrl?: string;
  /** `bedrock`: the AWS region. */
  region?: string;
  /** `bedrock`: a model id to the provider's id or inference profile (`claude-opus-5-5` → `global.anthropic.claude-opus-5-5`). */
  models?: Record<string, string>;
};

/** An account id the server takes. */
export const ACCOUNT_ID = /^[A-Za-z0-9_-]{1,40}$/;

/** The list a new runner starts with: the agent's default folder, whatever login it has. */
export const STARTER: Account[] = [{ id: "default", name: "This machine's login", provider: "claude", configDir: null }];

/** A list that is wrong, with why. */
export class AccountsError extends Error {}

/** Checks one account as written. */
export function checkAccount(a: unknown, at: string): Account {
  if (!a || typeof a !== "object") throw new AccountsError(`${at}: not an object`);
  const x = a as Record<string, unknown>;
  if (typeof x.id !== "string" || !ACCOUNT_ID.test(x.id)) throw new AccountsError(`${at}: id must be letters, digits, - and _`);
  if (typeof x.name !== "string" || !x.name) throw new AccountsError(`${at} (${x.id}): name is missing`);
  if (x.provider !== undefined && !["claude", "api", "bedrock"].includes(x.provider as string))
    throw new AccountsError(`${at} (${x.id}): provider must be claude, api or bedrock`);
  if (x.groups !== undefined && !(Array.isArray(x.groups) && x.groups.every((g) => typeof g === "string")))
    throw new AccountsError(`${at} (${x.id}): groups must be a list of group names`);
  if (x.models !== undefined && (typeof x.models !== "object" || x.models === null || Array.isArray(x.models)))
    throw new AccountsError(`${at} (${x.id}): models must be an object`);
  return x as Account;
}

/** Reads the accounts file, writing the starter list first when there is none. */
export function readAccounts(file: string): Account[] {
  if (!existsSync(file)) writeAccounts(file, STARTER);
  let raw: unknown;
  try {
    raw = JSON.parse(readFileSync(file, "utf8"));
  } catch (e) {
    throw new AccountsError(`${file} is not JSON: ${e instanceof Error ? e.message : String(e)}`);
  }
  if (!Array.isArray(raw)) throw new AccountsError(`${file} must hold a list of accounts`);
  const list = raw.map((a, i) => checkAccount(a, `${file} [${i}]`));
  const ids = new Set<string>();
  for (const a of list) {
    if (ids.has(a.id)) throw new AccountsError(`${file}: account ${a.id} is listed twice`);
    ids.add(a.id);
  }
  return list;
}

/** Writes the accounts file (only the owner may read it). */
export function writeAccounts(file: string, list: Account[]): void {
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, JSON.stringify(list, null, 2) + "\n", { mode: 0o600 });
}

/** The groups a session's work is in: those whose project or folder pattern matches. */
export function groupsOf(groups: Record<string, Group>, project: string | null | undefined, folder: string | null | undefined): string[] {
  const hit = (re: string | undefined, s: string | null | undefined) => !!re && !!s && new RegExp(re, "i").test(s);
  return Object.entries(groups)
    .filter(([, g]) => hit(g.projects, project) || hit(g.folders, folder))
    .map(([k]) => k);
}

/** Whether an account may take work in these groups: group work only on an account of one of them, other work only on one not kept to its groups. */
export const takes = (a: Account, inGroups: string[]): boolean =>
  a.enabled !== false && (inGroups.length ? inGroups.some((g) => a.groups?.includes(g)) : a.groupsOnly !== true);

/** A model id or inference profile only AWS Bedrock runs (`anthropic.…`, `us.anthropic.…`, `global.anthropic.…`, an ARN). */
export const isBedrockModel = (m?: string | null): boolean => !!m && /^(?:arn:aws:bedrock:|(?:[a-z]{2,7}\.)?anthropic\.)/.test(m);

/** The provider of an account. */
export const providerOf = (a: Account): Provider => a.provider ?? "claude";

/** Whether this account's agent can run the model: Bedrock only what it has an id for (its models, a Bedrock id, an alias). */
export function canRun(a: Account, m?: string | null): boolean {
  if (providerOf(a) !== "bedrock" || !m) return providerOf(a) === "bedrock" || !isBedrockModel(m);
  const base = m.replace(/\[1m\]$/i, "");
  return isBedrockModel(base) || /^(?:opus|sonnet|haiku|default|opusplan)$/i.test(base) || !!a.models?.[base];
}

/** The model as this account's agent takes it: on Bedrock a model id becomes its Bedrock id (`[1m]` kept). */
export function modelFor(a: Account, m: string): string {
  const wide = /\[1m\]$/i.test(m) ? "[1m]" : "",
    base = m.replace(/\[1m\]$/i, "");
  const mapped = a.models?.[base];
  return providerOf(a) === "bedrock" && !isBedrockModel(base) && mapped ? mapped + wide : m;
}
