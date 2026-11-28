// Pairing a runner with a server: a one-time code from the server's app (Machines, "Add a hut") is traded for the
// runner's name and token; the runner keeps the server's address in its settings and the token in its token file, and
// the server keeps the address it calls the runner at.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { platform } from "node:os";
import { advertised, CONFIG_FILE, NAME, readConfigFile, type Config } from "./config.ts";
import { VERSION } from "./version.ts";
import { SERVER } from "./wire.ts";

/** What pairing takes. */
export type PairAsk = { server: string; code: string; name?: string; place?: string; advertise?: string };

/** Trades the code for a name and token, then writes the settings and token file. Returns the name the server gave. */
export async function pair(c: Config, ask: PairAsk, f: typeof fetch = fetch): Promise<{ name: string; server: string; url: string }> {
  const server = ask.server.replace(/\/+$/, "");
  if (!/^https?:\/\/[^/\s]+/.test(server)) throw new Error(`the server's address must start with http:// or https:// (got ${server})`);
  const name = ask.name ?? c.name;
  if (!NAME.test(name)) throw new Error(`name ${JSON.stringify(name)}: letters, digits, - and _ only`);
  const url = (ask.advertise ?? advertised(c)).replace(/\/+$/, "");
  const r = await f(`${server}${SERVER.pair}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      code: ask.code.trim(),
      name,
      url,
      ...(ask.place ? { place: ask.place } : {}),
      platform: platform(),
      version: VERSION,
    }),
    signal: AbortSignal.timeout(30_000),
  }).catch((e: unknown) => {
    throw new Error(`the server at ${server} didn't answer (${e instanceof Error ? e.message : String(e)})`);
  });
  const answer = (await r.json().catch(() => ({}))) as { name?: unknown; token?: unknown; error?: unknown };
  if (!r.ok) throw new Error(typeof answer.error === "string" ? answer.error : `the server said ${r.status}`);
  if (typeof answer.token !== "string" || !answer.token) throw new Error("the server's answer had no token");
  const given = typeof answer.name === "string" && NAME.test(answer.name) ? answer.name : name;
  mkdirSync(dirname(c.tokenFile), { recursive: true, mode: 0o700 });
  writeFileSync(c.tokenFile, answer.token + "\n", { mode: 0o600 });
  const file = { ...readConfigFile(c.home), name: given, server, ...(ask.advertise ? { advertise: url } : {}) };
  mkdirSync(c.home, { recursive: true, mode: 0o700 });
  writeFileSync(join(c.home, CONFIG_FILE), JSON.stringify(file, null, 2) + "\n", { mode: 0o600 });
  return { name: given, server, url };
}
