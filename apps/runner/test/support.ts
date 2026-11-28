// Test support: a stand-in agent with the SDK's query shape, a stand-in server that records what the runner reports,
// a runner home in a temporary folder, and the server's contract fixtures.
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer, type IncomingMessage, type Server } from "node:http";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Options, Query } from "@anthropic-ai/claude-agent-sdk";
import { loadConfig, type Config } from "../src/config.ts";
import { startRunner, type Runner } from "../src/runner.ts";
import type { QueryFn } from "../src/sessions/engine.ts";

/** The server's contract fixtures. */
export const FIXTURES = new URL("../../../crates/emberhut-api/tests/fixtures/", import.meta.url);
/** A fixture as JSON. */
export const fixture = <T = unknown>(name: string): T => JSON.parse(readFileSync(new URL(name, FIXTURES), "utf8")) as T;

type Hook = (h: unknown) => Promise<unknown>;
type HookOptions = { hooks?: { PreToolUse?: { hooks: Hook[] }[] } };

/** Every query the stand-in agent was started with. */
export type Started = { options: Options; messages: string[] };

/**
 * A stand-in agent. Each message it gets plays one turn: an init, thinking, words typed and then whole, a shell command
 * and its output, a to-do list, and a result. A message with LIMIT ends on a usage limit; ASK asks the owner a question
 * (through the hook the runner gives the owner's sessions); SLOW waits until interrupted. LIMIT runs out only on the
 * first account, so a moved session carries on.
 */
export function standInAgent(): { query: QueryFn; started: Started[] } {
  const started: Started[] = [];
  const query: QueryFn = ({ prompt, options }) => {
    const me: Started = { options: options ?? {}, messages: [] };
    started.push(me);
    const out: unknown[] = [];
    let wake: (() => void) | null = null,
      closed = false,
      interrupted: (() => void) | null = null;
    const push = (m: unknown) => {
      out.push(m);
      wake?.();
    };
    const sid = options?.sessionId ?? options?.resume ?? "s";
    let inited = false;
    const playing = (async () => {
      for await (const m of prompt) {
        const text = typeof m.message.content === "string" ? m.message.content : "";
        me.messages.push(text);
        const env = (options as { env?: Record<string, string> } | undefined)?.env ?? {};
        if (!inited) {
          inited = true;
          // Its transcript, where the agent keeps one: in its account folder's projects.
          if (env.CLAUDE_CONFIG_DIR) {
            mkdirSync(join(env.CLAUDE_CONFIG_DIR, "projects", "-work"), { recursive: true });
            writeFileSync(join(env.CLAUDE_CONFIG_DIR, "projects", "-work", `${sid}.jsonl`), "{}\n", { flag: "a" });
          }
          push({ type: "system", subtype: "init", session_id: sid });
        }
        if (text.includes("SLOW")) {
          await new Promise<void>((r) => (interrupted = r));
          push({ type: "result", subtype: "error_during_execution", is_error: true, result: "" });
          continue;
        }
        if (text.includes("LIMIT") && /acct-1$/.test(env.CLAUDE_CONFIG_DIR ?? "")) {
          push({ type: "assistant", message: { content: [{ type: "text", text: "Working on it" }] } });
          push({ type: "result", subtype: "success", is_error: true, result: "You've hit your limit · resets 5pm" });
          continue;
        }
        if (text.includes("ASK")) {
          const hook = (options as HookOptions).hooks?.PreToolUse?.[0]?.hooks[0];
          await hook?.({
            tool_use_id: "call-ask",
            tool_input: {
              questions: [
                {
                  question: "Which one?",
                  header: "Pick",
                  options: [{ label: "The first" }, { label: "The second", description: "Slower" }],
                },
              ],
            },
          });
          push({ type: "result", subtype: "success", is_error: false, result: "Asked." });
          continue;
        }
        push({ type: "stream_event", event: { type: "content_block_start", content_block: { type: "thinking" } } });
        push({ type: "stream_event", event: { type: "content_block_delta", delta: { type: "thinking_delta", thinking: "Look first" } } });
        push({ type: "stream_event", event: { type: "content_block_start", content_block: { type: "text" } } });
        push({ type: "stream_event", event: { type: "content_block_delta", delta: { type: "text_delta", text: "Hel" } } });
        push({ type: "stream_event", event: { type: "message_delta", usage: { output_tokens: 12 } } });
        push({
          type: "assistant",
          message: {
            content: [
              { type: "thinking", thinking: "Look first" },
              { type: "text", text: "Hello" },
            ],
          },
        });
        push({
          type: "assistant",
          message: {
            content: [{ type: "tool_use", id: "call-1", name: "Bash", input: { command: "npm test", description: "Run the tests" } }],
          },
        });
        push({
          type: "user",
          message: { content: [{ type: "tool_result", tool_use_id: "call-1", content: "ok\n2 passed", is_error: false }] },
        });
        push({
          type: "assistant",
          message: {
            content: [
              { type: "tool_use", id: "call-2", name: "Edit", input: { file_path: "/w/a.ts", old_string: "old", new_string: "new" } },
            ],
          },
        });
        push({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "call-2", content: "edited", is_error: false }] } });
        push({
          type: "assistant",
          message: {
            content: [
              {
                type: "tool_use",
                id: "call-3",
                name: "TodoWrite",
                input: { todos: [{ content: "Test", status: "in_progress", activeForm: "Testing" }] },
              },
            ],
          },
        });
        push({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "call-3", content: "ok", is_error: false }] } });
        push({
          type: "result",
          subtype: "success",
          is_error: false,
          result: "All green.\nSTATUS: done",
          usage: { input_tokens: 10, output_tokens: 12 },
        });
      }
    })();
    playing.catch(() => undefined);
    const iterator: AsyncIterator<unknown> = {
      next: async () => {
        while (!out.length && !closed) await new Promise<void>((r) => (wake = r));
        if (out.length) return { value: out.shift(), done: false };
        return { value: undefined, done: true };
      },
    };
    const q = {
      [Symbol.asyncIterator]: () => iterator,
      interrupt: () => {
        interrupted?.();
        interrupted = null;
        return Promise.resolve();
      },
      close: () => {
        closed = true;
        wake?.();
      },
    };
    return q as unknown as Query;
  };
  return { query, started };
}

/** What the stand-in server got. */
export type Got = { path: string; auth: string | undefined; body: Record<string, unknown> };

/** A stand-in server: records the runner's events, samples and asks; answers pairing with a fixed token. */
export async function standInServer(token: string): Promise<{ url: string; got: Got[]; close: () => Promise<void> }> {
  const got: Got[] = [];
  const read = async (req: IncomingMessage) => {
    let raw = "";
    for await (const c of req as AsyncIterable<Buffer>) raw += c.toString("utf8");
    return raw ? (JSON.parse(raw) as Record<string, unknown>) : {};
  };
  const server: Server = createServer((req, res) => {
    read(req)
      .then((body) => {
        got.push({ path: req.url ?? "", auth: req.headers.authorization, body });
        res.setHeader("Content-Type", "application/json");
        if (req.url === "/api/runner/pair") {
          if (body.code !== "PAIR-1234") return void res.writeHead(401).end(JSON.stringify({ error: "that code isn't valid any more" }));
          return void res.end(JSON.stringify({ name: body.name, token }));
        }
        if (req.headers.authorization !== `Bearer ${token}`)
          return void res.writeHead(401).end(JSON.stringify({ error: "runners use their token" }));
        if (req.url === "/api/runner/mcp")
          return void res.end(JSON.stringify({ mcpServers: { docs: { type: "http", url: "https://mcp.example.com" } } }));
        res.end(JSON.stringify({ ok: true }));
      })
      .catch(() => res.writeHead(400).end());
  });
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  const { port } = server.address() as { port: number };
  return { url: `http://127.0.0.1:${port}`, got, close: () => new Promise<void>((ok) => server.close(() => ok())) };
}

/** A runner home in a temporary folder, with a settings file, a token, and accounts logged in (credential files). */
export function tempHome(o: { server?: string; token?: string; config?: Record<string, unknown>; accounts?: unknown[] } = {}): {
  home: string;
  work: string;
  config: () => Config;
  cleanup: () => void;
} {
  // Inside the home folder (a temporary one in the tests), as the folder pickers only look there.
  const root = mkdtempSync(join(homedir(), "emberhut-runner-test-"));
  const home = join(root, "home"),
    work = join(root, "work");
  mkdirSync(home, { recursive: true });
  mkdirSync(work, { recursive: true });
  const accounts = o.accounts ?? [
    { id: "a-1", name: "First account", configDir: join(root, "acct-1"), order: 1 },
    { id: "a-2", name: "Second account", configDir: join(root, "acct-2"), order: 2 },
  ];
  for (const a of accounts as { configDir?: string }[])
    if (a.configDir) {
      mkdirSync(a.configDir, { recursive: true });
      writeFileSync(join(a.configDir, ".credentials.json"), "{}");
    }
  writeFileSync(join(home, "accounts.json"), JSON.stringify(accounts));
  writeFileSync(
    join(home, "config.json"),
    JSON.stringify({
      name: "test-runner",
      port: 0,
      worktreesDir: join(root, "worktrees"),
      ...(o.server ? { server: o.server } : {}),
      ...o.config,
    }),
  );
  if (o.token) writeFileSync(join(home, "token"), o.token);
  return {
    home,
    work,
    config: () => loadConfig({ EMBERHUT_RUNNER_HOME: home }),
    cleanup: () => rmSync(root, { recursive: true, force: true }),
  };
}

/** Starts a runner on a temporary home with the stand-in agent; no usage probes. */
export async function testRunner(h: ReturnType<typeof tempHome>, query: QueryFn): Promise<Runner> {
  return startRunner(h.config(), { query, credentialFiles: true, noProbe: true, tmp: h.work });
}

/** Waits until `ok()` holds (polling every 10 ms, 5 s at most). */
export async function until(ok: () => boolean, what = "the condition"): Promise<void> {
  const end = Date.now() + 5_000;
  while (!ok()) {
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 10));
  }
}
