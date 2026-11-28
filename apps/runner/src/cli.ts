// emberhut-runner: the runner's command line. `run` is the daemon (what the service starts); the rest set it up on this
// machine: pairing with a server, the service, the accounts, and a look at whether it answers.
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { arch, platform } from "node:os";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";
import { Accounts } from "./accounts/index.ts";
import { ACCOUNT_ID, readAccounts, writeAccounts, type Account, type Provider } from "./accounts/list.ts";
import { dirOf, prepareDir } from "./accounts/login.ts";
import { loadConfig, readToken, type Config } from "./config.ts";
import { log, messageOf } from "./log.ts";
import { pair } from "./pair.ts";
import { startRunner } from "./runner.ts";
import * as service from "./service.ts";
import { VERSION } from "./version.ts";

const HELP = `emberhut-runner ${VERSION}: runs coding-agent sessions on this machine for an Emberhut server.

Usage:
  emberhut-runner run                         the daemon (what the service starts)
  emberhut-runner pair --server URL --code CODE [--name NAME] [--place WORDS] [--advertise URL] [--no-service]
                                              pair with a server (a code from the app's Machines page), then install the service
  emberhut-runner service install|uninstall|restart|status [--force]
  emberhut-runner status                      whether the runner here answers, and with what
  emberhut-runner paired                      exits 0 when paired (for installers)
  emberhut-runner accounts [list]             the accounts, their use and logins
  emberhut-runner accounts add ID "NAME" [--provider claude|api|bedrock] [--config-dir DIR] [--order N]
  emberhut-runner accounts login ID           sign an account in (the agent's own sign-in, in its folder)
  emberhut-runner accounts check              look at every account's use now
  emberhut-runner accounts clear ID           forget that an account was out
  emberhut-runner version

Settings: config.json in the runner's home (EMBERHUT_RUNNER_HOME; see the runner's README).`;

const out = (s: string) => process.stdout.write(s + "\n");
const pct = (x: unknown) => (typeof x === "number" ? `${String(Math.round(x)).padStart(3)}%` : "   ?");

function showAccounts(a: Accounts): void {
  for (const x of a.summary()) {
    const state = !x.enabled
      ? "off"
      : !x.loggedIn
        ? `not logged in: emberhut-runner accounts login ${x.id}`
        : x.error
          ? `error: ${x.error}`
          : x.limitedUntil
            ? `out until ${x.limitedUntil.slice(0, 16)} (${x.limitedWhy ?? ""})`
            : "ok";
    out(
      `${x.id.padEnd(12)} ${x.name.padEnd(28)} ${(x.email ?? "").padEnd(26)} 5h ${pct(x.used5h)}  week ${pct(x.used7d)}  ${x.provider.padEnd(8)} ${state}`,
    );
  }
}

/** The agent's own executable, as the SDK ships it for this platform (else `claude` on the PATH). */
function agentExecutable(): string {
  try {
    const sdk = createRequire(import.meta.url).resolve("@anthropic-ai/claude-agent-sdk");
    const pkg = createRequire(sdk).resolve(`@anthropic-ai/claude-agent-sdk-${platform()}-${arch()}/package.json`);
    return join(dirname(pkg), platform() === "win32" ? "claude.exe" : "claude");
  } catch {
    return "claude";
  }
}

async function accountsCommand(c: Config, args: string[]): Promise<number> {
  const a = new Accounts(c);
  const [sub, id, name] = args;
  try {
    if (!sub || sub === "list") showAccounts(a);
    else if (sub === "check") {
      await a.probeAll();
      showAccounts(a);
    } else if (sub === "clear" && id) {
      a.clearLimit(id);
      showAccounts(a);
    } else if (sub === "login" && id) {
      const acc = a.get(id);
      if (!acc)
        throw new Error(
          `no account ${id}; accounts: ${a
            .list()
            .map((x) => x.id)
            .join(", ")}`,
        );
      if ((acc.provider ?? "claude") !== "claude")
        throw new Error(
          `${acc.name} signs in with a key: put it in ${join(c.home, "secrets", acc.keyFile ?? "(its keyFile)")} (only you may read it)`,
        );
      prepareDir(acc, c.skillsDir);
      out(
        `Signing in "${acc.name}" (folder ${dirOf(acc)}). Open the link that comes next, sign in with the right account, and paste the code back here.`,
      );
      const r = spawnSync(agentExecutable(), ["auth", "login"], { stdio: "inherit", env: a.logins.envFor(acc, c.skillsDir) });
      if (r.status !== 0) throw new Error("the sign-in didn't finish; run the same command again");
      a.logins.forget(acc.id);
      const u = await a.probe(acc);
      out(u?.email ? `Signed in as ${u.email}.` : `Not signed in yet (${u?.error ?? "unknown"}).`);
    } else if (sub === "add" && id && name) {
      const { values } = parseArgs({
        args: args.slice(3),
        options: { provider: { type: "string" }, "config-dir": { type: "string" }, order: { type: "string" } },
      });
      if (!ACCOUNT_ID.test(id)) throw new Error("an account id is letters, digits, - and _");
      const list = readAccounts(c.accountsFile);
      if (list.some((x) => x.id === id)) throw new Error(`${id} is already listed`);
      const provider = (values.provider ?? "claude") as Provider;
      if (!["claude", "api", "bedrock"].includes(provider)) throw new Error("provider: claude, api or bedrock");
      const acc: Account = {
        id,
        name,
        provider,
        ...(provider === "claude" ? { configDir: values["config-dir"] ?? `~/.claude-${id}` } : {}),
        ...(values.order ? { order: Number(values.order) } : {}),
      };
      writeAccounts(c.accountsFile, [...list, acc]);
      out(
        provider === "claude"
          ? `Added. Now: emberhut-runner accounts login ${id}`
          : `Added. Put its key in ${join(c.home, "secrets")} (see the README).`,
      );
    } else {
      out(HELP);
      return 1;
    }
    return 0;
  } finally {
    a.close();
  }
}

async function statusCommand(c: Config): Promise<number> {
  const base = `http://${c.listen[0]!.includes(":") ? `[${c.listen[0]!}]` : c.listen[0]!}:${c.port}`;
  const h = await fetch(`${base}/health`, { signal: AbortSignal.timeout(5_000) }).catch(() => null);
  if (!h?.ok) {
    out(`The runner at ${base} doesn't answer.\n${service.status()}`);
    return 1;
  }
  out(`The runner at ${base} answers: ${await h.text()}`);
  const token = readToken(c);
  out(
    token ? `Paired with ${c.server ?? "(no server set)"} as ${c.name}.` : "Not paired yet: emberhut-runner pair --server URL --code CODE",
  );
  return 0;
}

/** Runs one command; returns its exit status. */
export async function main(argv: string[]): Promise<number> {
  const [cmd, ...rest] = argv;
  if (!cmd || cmd === "help" || cmd === "--help" || cmd === "-h") {
    out(HELP);
    return cmd ? 0 : 1;
  }
  if (cmd === "version" || cmd === "--version") {
    out(VERSION);
    return 0;
  }
  const c = loadConfig();
  switch (cmd) {
    case "run": {
      const r = await startRunner(c);
      const stop = (sig: string) => {
        log(`${sig}: stopping; running turns carry on after the next start`);
        r.close().then(
          () => process.exit(0),
          () => process.exit(1),
        );
      };
      process.on("SIGTERM", () => stop("SIGTERM"));
      process.on("SIGINT", () => stop("SIGINT"));
      return await new Promise<number>(() => {});
    }
    case "pair": {
      const { values } = parseArgs({
        args: rest,
        options: {
          server: { type: "string" },
          code: { type: "string" },
          name: { type: "string" },
          place: { type: "string" },
          advertise: { type: "string" },
          "no-service": { type: "boolean" },
        },
      });
      const server = values.server ?? process.env.EMBERHUT_SERVER,
        code = values.code ?? process.env.EMBERHUT_PAIR_CODE;
      if (!server || !code) {
        out("pair needs --server URL and --code CODE (the code from the app's Machines page)");
        return 1;
      }
      const p = await pair(c, {
        server,
        code,
        ...(values.name ? { name: values.name } : {}),
        ...(values.place ? { place: values.place } : {}),
        ...(values.advertise ? { advertise: values.advertise } : {}),
      });
      out(`Paired with ${p.server} as ${p.name}; the server calls this runner at ${p.url}.`);
      if (!values["no-service"]) out(service.install(loadConfig()));
      return 0;
    }
    case "service": {
      const force = rest.includes("--force");
      const sub = rest.find((x) => !x.startsWith("--"));
      if (sub === "install") out(service.install(c, force));
      else if (sub === "uninstall") out(service.uninstall(c));
      else if (sub === "restart") out(service.restart());
      else if (sub === "status") out(service.status());
      else {
        out(HELP);
        return 1;
      }
      return 0;
    }
    case "status":
      return statusCommand(c);
    case "paired":
      // For installers: 0 when this runner has a token and a server.
      out(readToken(c) && c.server ? `paired with ${c.server} as ${c.name}` : "not paired");
      return readToken(c) && c.server ? 0 : 1;
    case "accounts":
      return accountsCommand(c, rest);
    default:
      out(HELP);
      return 1;
  }
}

// Run as `node src/cli.ts …` (the bin shim calls main itself).
if (/[/\\]cli\.ts$/.test(process.argv[1] ?? "")) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (e: unknown) => {
      process.stderr.write(`emberhut-runner: ${messageOf(e)}\n`);
      process.exit(1);
    },
  );
}
