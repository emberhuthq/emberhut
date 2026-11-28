// One runner: its store, accounts, reporter, sessions, machine samples and API, put together and started. Used by the
// `run` command and, with a stand-in agent and server, by the tests.
import type { Server } from "node:http";
import { mkdirSync } from "node:fs";
import { query as sdkQuery } from "@anthropic-ai/claude-agent-sdk";
import { Accounts } from "./accounts/index.ts";
import { readToken, type Config } from "./config.ts";
import { Files } from "./files.ts";
import { listen } from "./http.ts";
import { inBackground, log } from "./log.ts";
import { startSampling } from "./machine.ts";
import { Reporter } from "./report.ts";
import { Sessions, type QueryFn } from "./sessions/engine.ts";
import { Store, now } from "./store.ts";

/** What a test may put in place of the real thing. */
export type RunnerOptions = {
  /** The agent (the SDK's `query` by default). */
  query?: QueryFn;
  /** How the runner reaches its server. */
  fetch?: typeof fetch;
  /** Look only at credential files for logins, never the macOS Keychain. */
  credentialFiles?: boolean;
  /** Don't look at the accounts' use (no agent process for it). */
  noProbe?: boolean;
  /** Where files may come from besides the sessions' folders. */
  tmp?: string;
};

/** A started runner. */
export type Runner = {
  config: Config;
  store: Store;
  accounts: Accounts;
  sessions: Sessions;
  reporter: Reporter;
  servers: Server[];
  /** The port it listens on (the configured one, or the one picked for port 0). */
  port: number;
  /** Stops everything; running turns resume at the next start. */
  close: () => Promise<void>;
};

/** Starts a runner with these settings. */
export async function startRunner(config: Config, o: RunnerOptions = {}): Promise<Runner> {
  mkdirSync(config.home, { recursive: true, mode: 0o700 });
  const store = new Store(config.home);
  const accounts = new Accounts(config, o.credentialFiles === undefined ? {} : { credentialFiles: o.credentialFiles });
  const token = () => readToken(config);
  const server = () => config.server;
  const reporter = new Reporter(store, { server, token, name: config.name, ...(o.fetch ? { fetch: o.fetch } : {}) });
  const files = new Files(store, config.home, config.name, config.maxFileBytes, o.tmp);
  const sessions = new Sessions(store, accounts, reporter, files, config, o.query ?? sdkQuery);
  // The first MCP answer before boot resumes what a restart cut off, so those turns open with them too (15 s at most).
  if (config.shareMcp) await reporter.loadMcp();
  const mcpTimer = setInterval(() => inBackground("asking for MCP servers", reporter.loadMcp()), 10 * 60_000);
  mcpTimer.unref();
  const sampling = startSampling({
    server,
    token,
    machine: config.name,
    sessions: () => sessions.summary(),
    every: config.machineEvery,
    ...(o.fetch ? { fetch: o.fetch } : {}),
  });
  const servers = await listen(
    { name: config.name, token, store, sessions, accounts, files, machine: sampling.latest, jobLabels: config.jobLabels, started: now() },
    config.listen,
    config.port,
  );
  sessions.boot();
  inBackground("sending events", reporter.flush());
  let probeTimer: NodeJS.Timeout | null = null;
  if (!o.noProbe) {
    const probe = () => {
      probeTimer = setTimeout(() => {
        accounts
          .probeAll()
          .catch((e: unknown) => log("usage check failed:", e instanceof Error ? e.message : String(e)))
          .finally(probe);
      }, 10 * 60_000);
      probeTimer.unref();
    };
    setTimeout(() => {
      accounts
        .probeAll()
        .catch((e: unknown) => log("usage check failed:", e instanceof Error ? e.message : String(e)))
        .finally(probe);
    }, 2_000).unref();
  }
  const address = servers[0]?.address();
  return {
    config,
    store,
    accounts,
    sessions,
    reporter,
    servers,
    port: typeof address === "object" && address ? address.port : config.port,
    close: async () => {
      clearInterval(mcpTimer);
      if (probeTimer) clearTimeout(probeTimer);
      sampling.stop();
      reporter.stop();
      sessions.shutdown();
      const closed = Promise.all(servers.map((s) => new Promise<void>((ok) => s.close(() => ok()))));
      for (const s of servers) s.closeAllConnections();
      await closed;
      accounts.close();
      store.close();
    },
  };
}
