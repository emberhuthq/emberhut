// Picking an account for a session, and knowing how much each has left. A session starts on the account with the most
// room that may take its work; when its turn ends on a usage limit, the account is out until it resets and the session
// moves on. The numbers come from what the agent's own usage view shows (asked every 10 minutes, as that account) and
// from every rate-limit event a running session sees.
import { DatabaseSync, type SQLInputValue } from "node:sqlite";
import { join } from "node:path";
import { query } from "@anthropic-ai/claude-agent-sdk";
import type { Config } from "../config.ts";
import { secretsDir } from "../config.ts";
import { canRun, groupsOf, isBedrockModel, providerOf, readAccounts, takes, type Account } from "./list.ts";
import { keyFileOf, Logins, secret } from "./login.ts";

/** An account's use as last seen. Percent figures are 0-100. */
export type Use = {
  id: string;
  email: string | null;
  plan: string | null;
  used5h: number | null;
  used7d: number | null;
  resets5h: string | null;
  resets7d: string | null;
  limited_until: string | null;
  limited_why: string | null;
  checked: string | null;
  error: string | null;
};

/** What a session's work is, for picking its account. */
export type Work = {
  project?: string | null;
  /** The folder it was asked to start in. */
  cwd?: string | null;
  /** It asked for an opt-in account (by id, or a model only one runs): those go first. */
  optIn?: boolean;
  /** Only these accounts, no other (a pinned session). */
  only?: string[] | undefined;
  /** The model it runs. */
  model?: string | null;
  /** Accounts left out (the one it just ran out on). */
  exclude?: string[];
  /** Stay on this one while it has room, so nothing moves mid-run. */
  prefer?: string | null;
};

/** An account that can take work now, and its room (0-100). */
export type Ranked = { account: Account; use: Use | null; room: number };

/** A rate-limit event's numbers, as the agent reports them. */
export type RateLimitInfo = {
  status?: string;
  overageStatus?: string;
  resetsAt?: number;
  rateLimitType?: string;
  unifiedWindows?: Record<string, { utilization?: number; resetsAt?: number } | undefined>;
};

/** An account as the server reads it (`GET /accounts`). */
export type AccountView = {
  id: string;
  name: string;
  scope: string;
  order: number | null;
  enabled: boolean;
  loggedIn: boolean;
  email: string | null;
  plan: string | null;
  provider: string;
  optIn: boolean;
  used5h: number | null;
  used7d: number | null;
  resets5h: string | null;
  resets7d: string | null;
  limitedUntil: string | null;
  limitedWhy: string | null;
  checked: string | null;
  error: string | null;
};

/** An answer from a usage probe (the agent's usage view). */
type UsageAnswer = { rate_limits?: Record<string, { utilization?: number; resets_at?: string } | undefined> };

/** A result that says the turn ended on a usage limit. */
export const LIMIT =
  /hit your (?:\w+ )?limit|usage limit reached|out of (?:extra )?usage|rate limit(?:ed)?\b.*(?:reset|try again)|limit · resets/i;
/** A provider's own refusal (throttled, over quota, the key refused, the service down): the session moves on as at a limit. */
export const PROVIDER_DOWN =
  /throttl|too many (?:tokens|requests)|\b(?:429|500|502|503|529)\b|service ?unavailable|overloaded|access ?denied|not authori[sz]ed|unrecognized ?client|security token|bearer token|invalid api key|marketplace|subscription|model access|on-demand throughput|quota|credits?\b|expired/i;

const WRONG = "logged in as another account";
const now = () => new Date().toISOString();
const iso = (secs?: number | null) => (secs ? new Date(secs * 1000).toISOString() : null);

/** The runner's accounts: the list, their use and the pick. */
export class Accounts {
  private readonly db: DatabaseSync;
  private readonly config: Pick<Config, "accountsFile" | "home" | "groups" | "defaultAccount" | "skillsDir">;
  /** Logins and each session's environment. */
  readonly logins: Logins;

  constructor(
    config: Pick<Config, "accountsFile" | "home" | "groups" | "defaultAccount" | "skillsDir">,
    opts: { db?: string; credentialFiles?: boolean } = {},
  ) {
    this.config = config;
    this.db = new DatabaseSync(opts.db ?? join(config.home, "accounts.db"));
    this.db.exec(`pragma journal_mode = wal; pragma synchronous = normal; pragma busy_timeout = 10000;
      create table if not exists usage (id text primary key, email text, plan text, used5h real, used7d real, resets5h text, resets7d text,
        limited_until text, limited_why text, checked text, error text)`);
    this.logins = new Logins(secretsDir(config), opts.credentialFiles);
  }

  /** The list, read each time (the owner may edit it while the runner runs). */
  list(): Account[] {
    return readAccounts(this.config.accountsFile);
  }
  /** One account. */
  get(id: string | null | undefined): Account | null {
    return id ? (this.list().find((a) => a.id === id) ?? null) : null;
  }
  /** The default account, when it names a listed, enabled one. */
  defaultAccount(): string | null {
    const id = this.config.defaultAccount;
    const a = this.get(id);
    return a && a.enabled !== false ? a.id : null;
  }
  /** The groups a session's work is in. */
  groupsOf(project?: string | null, cwd?: string | null): string[] {
    return groupsOf(this.config.groups, project, cwd);
  }

  /** Its use as last seen. */
  useOf(id: string): Use | null {
    return (this.db.prepare("select * from usage where id = ?").get(id) as Use | undefined) ?? null;
  }
  private save(id: string, v: Partial<Use>): void {
    this.db.prepare("insert or ignore into usage(id) values (?)").run(id);
    for (const [k, x] of Object.entries(v))
      if (x !== undefined) this.db.prepare(`update usage set ${k} = ? where id = ?`).run(x as SQLInputValue, id);
  }
  /** Whether an account is out now. */
  limited(u: Use | null): boolean {
    return !!u?.limited_until && Date.parse(u.limited_until) > Date.now();
  }

  /** What a running session saw: the windows' use, and a refusal (out until it resets). */
  observe(id: string, info: RateLimitInfo | undefined): void {
    const w = info?.unifiedWindows ?? {},
      pct = (x?: number) => (x == null ? undefined : Math.round(x * 1000) / 10);
    this.save(id, {
      used5h: pct(w.five_hour?.utilization),
      used7d: pct(w.seven_day?.utilization),
      resets5h: iso(w.five_hour?.resetsAt) ?? undefined,
      resets7d: iso(w.seven_day?.resetsAt) ?? undefined,
      checked: now(),
    });
    if (info?.status === "rejected" && info.overageStatus !== "allowed")
      this.markLimited(id, iso(info.resetsAt), `refused: ${info.rateLimitType ?? "limit"}`);
  }
  /** Out until `until` (no time known: an hour, then it's looked at again). */
  markLimited(id: string, until: string | null, why: string): void {
    this.save(id, { limited_until: until ?? new Date(Date.now() + 3_600_000).toISOString(), limited_why: why });
  }
  /** Forgets that it was out. */
  clearLimit(id: string): void {
    this.db.prepare("update usage set limited_until = null, limited_why = null where id = ?").run(id);
  }

  /** Looks at one account's use now, as the agent's usage view shows it, without a turn or a model call. */
  async probe(a: Account): Promise<Use | null> {
    if (providerOf(a) !== "claude") return this.probeKey(a);
    if (!this.logins.loggedIn(a)) {
      this.save(a.id, { error: "not logged in", checked: now() });
      return this.useOf(a.id);
    }
    // A prompt that never sends anything: the process only answers the questions below.
    const nothing: AsyncIterable<never> = { [Symbol.asyncIterator]: () => ({ next: () => new Promise<never>(() => {}) }) };
    const q = query({
      prompt: nothing,
      options: {
        cwd: this.config.home,
        env: this.logins.envFor(a, this.config.skillsDir),
        strictMcpConfig: true,
        settingSources: [],
        model: "haiku",
      },
    });
    const timeout = new Promise<never>((_, no) => setTimeout(() => no(new Error("no answer in 30 s")), 30_000).unref());
    try {
      const info = await Promise.race([q.accountInfo(), timeout]);
      if (!info?.email) {
        this.save(a.id, { error: "not logged in", checked: now() });
        return this.useOf(a.id);
      }
      if (a.email && info.email.toLowerCase() !== a.email.toLowerCase()) {
        this.save(a.id, { email: info.email, error: `${WRONG}: ${info.email}, not ${a.email}`, checked: now() });
        return this.useOf(a.id);
      }
      this.save(a.id, { email: info.email, plan: info.subscriptionType ?? null, error: null, checked: now() });
      // The usage view is marked experimental in the SDK; when it changes, the rate-limit events still keep the numbers.
      const usage = (q as unknown as { usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET?: (o: object) => Promise<UsageAnswer> })
        .usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET;
      if (usage) {
        const u = await Promise.race([usage.call(q, { skipBehaviors: true }), timeout]);
        const r = u?.rate_limits ?? {};
        this.save(a.id, {
          used5h: r.five_hour?.utilization,
          used7d: r.seven_day?.utilization,
          resets5h: r.five_hour?.resets_at,
          resets7d: r.seven_day?.resets_at,
        });
      }
      // A window it was refused in that has reset is open again.
      const u2 = this.useOf(a.id);
      if (u2?.limited_until && (u2.used5h ?? 0) < 98 && (u2.used7d ?? 0) < 98) this.clearLimit(a.id);
    } catch (e) {
      this.save(a.id, { error: String(e instanceof Error ? e.message : e).slice(0, 160), checked: now() });
    } finally {
      q.close();
    }
    return this.useOf(a.id);
  }

  /** A per-token provider has no windows: its look is that its key (and Bedrock's region) are there. */
  private probeKey(a: Account): Use | null {
    const plan = providerOf(a) === "bedrock" ? "aws-bedrock" : "api";
    const ok = !!secret(secretsDir(this.config), keyFileOf(a)) && (providerOf(a) !== "bedrock" || !!a.region);
    this.save(a.id, { plan, error: ok ? null : "not logged in", checked: now() });
    return this.useOf(a.id);
  }

  /** Looks at every enabled account's use, one after the other. */
  async probeAll(): Promise<void> {
    for (const a of this.list().filter((x) => x.enabled !== false)) await this.probe(a);
  }

  /** Room left, 0-100, counting a window that resets within the hour (5-hour) or 12 hours (week) as fresh. */
  static room(u: Use | null): number {
    const soon = (at: string | null | undefined, ms: number) => !!at && Date.parse(at) - Date.now() < ms;
    const five = soon(u?.resets5h, 3_600_000) ? 0 : (u?.used5h ?? 0),
      week = soon(u?.resets7d, 12 * 3_600_000) ? 0 : (u?.used7d ?? 0);
    return 100 - Math.max(five, week);
  }

  /** The accounts a pinned session or a provider-only model may run on (none: any). */
  onlyFor(pin: string | null | undefined, model: string | null | undefined): string[] | undefined {
    if (pin) return [pin];
    return isBedrockModel(model)
      ? this.list()
          .filter((a) => providerOf(a) === "bedrock")
          .map((a) => a.id)
      : undefined;
  }

  /**
   * The accounts that can take this work now, best first: allowed for its groups, logged in, not out, able to run its
   * model. Opt-in accounts go first for work that asks for them and last otherwise; the default account goes before the
   * rest; nearly full ones (under 10% left) after the others; then the one it is on, the lowest order and the most room.
   */
  ranked(w: Work = {}): Ranked[] {
    const def = this.defaultAccount(),
      inGroups = this.groupsOf(w.project, w.cwd);
    const first = (a: Account) => Number(a.id !== def);
    const side = (a: Account) => Number(a.optIn === true && a.id !== def) ^ Number(!!w.optIn);
    return this.list()
      .filter(
        (a) =>
          takes(a, inGroups) &&
          !w.exclude?.includes(a.id) &&
          (!w.only || w.only.includes(a.id)) &&
          canRun(a, w.model) &&
          this.logins.loggedIn(a),
      )
      .map((a) => ({ account: a, use: this.useOf(a.id), room: Accounts.room(this.useOf(a.id)) }))
      .filter((p) => !this.limited(p.use) && p.use?.error !== "not logged in" && !p.use?.error?.startsWith(WRONG))
      .sort(
        (x, y) =>
          side(x.account) - side(y.account) ||
          first(x.account) - first(y.account) ||
          Number(x.room < 10) - Number(y.room < 10) ||
          Number(y.account.id === w.prefer) - Number(x.account.id === w.prefer) ||
          (x.account.order ?? 1e9) - (y.account.order ?? 1e9) ||
          y.room - x.room,
      );
  }
  /** The best account for this work now, or null. */
  pick(w: Work = {}): Account | null {
    return this.ranked(w)[0]?.account ?? null;
  }
  /** When the first account that could take this work is free again. */
  nextFree(w: Work = {}): string | null {
    const inGroups = this.groupsOf(w.project, w.cwd);
    return (
      this.list()
        .filter((a) => takes(a, inGroups) && (!w.only || w.only.includes(a.id)) && this.logins.loggedIn(a))
        .map((a) => this.useOf(a.id)?.limited_until)
        .filter((x): x is string => !!x)
        .sort()[0] ?? null
    );
  }
  /** Why this machine never takes the work, or null: it is in groups no account here takes. */
  refusal(project?: string | null, cwd?: string | null): string | null {
    const inGroups = this.groupsOf(project, cwd);
    if (!inGroups.length || this.list().some((a) => takes(a, inGroups))) return null;
    return `no account on this runner takes ${inGroups.join(" or ")} work: start it on a runner that has one`;
  }

  /** Why an account isn't picked now, in words. */
  lowWhy(id: string): string {
    const a = this.get(id),
      u = this.useOf(id),
      n = a?.name ?? id;
    if (this.limited(u)) return `${n} is out until ${new Date(u!.limited_until!).toLocaleString()}`;
    if (!a || !this.logins.loggedIn(a) || u?.error === "not logged in" || u?.error?.startsWith(WRONG)) return `${n} isn't logged in`;
    const five = u?.used5h ?? 0,
      week = u?.used7d ?? 0;
    return week >= five ? `${n} is at ${Math.round(week)}% for the week` : `${n} is at ${Math.round(five)}% of its 5 hours`;
  }

  /** Every account as the server reads it (`GET /accounts`): never a token or key. */
  summary(): AccountView[] {
    return this.list().map((a) => {
      const u = this.useOf(a.id),
        out = this.limited(u);
      return {
        id: a.id,
        name: a.name,
        // "any" is offered for new sessions; an account kept to its groups' work is named by them.
        scope: a.groupsOnly && a.groups?.length ? `groups:${a.groups.join(",")}` : "any",
        order: a.order ?? null,
        enabled: a.enabled !== false,
        loggedIn: this.logins.loggedIn(a),
        email: u?.email ?? null,
        plan: u?.plan ?? null,
        provider: providerOf(a),
        optIn: a.optIn === true,
        used5h: u?.used5h ?? null,
        used7d: u?.used7d ?? null,
        resets5h: u?.resets5h ?? null,
        resets7d: u?.resets7d ?? null,
        limitedUntil: out ? u!.limited_until : null,
        limitedWhy: out ? u!.limited_why : null,
        checked: u?.checked ?? null,
        error: u?.error ?? null,
      };
    });
  }

  /** Closes the use database. */
  close(): void {
    this.db.close();
  }
}
