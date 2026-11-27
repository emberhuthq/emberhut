// The socket to the owner's server (`<server>/api/chrome/ws`), one per Chrome profile. The server's address comes from
// pairing only. The first message pairs (a link's code) or says hello (this profile's token), naming the protocol the
// extension speaks; after the server's hello, its calls are answered (relay.ts) and its messages acted on.

import { TICK_MS, fromBeforeSleep, isStale, retryAt as nextTry, slept } from "../backoff";
import { firstMessage } from "../first-message";
import { inBackground, messageOf } from "../log";
import { readMessage, type Message } from "../messages";
import type { About } from "../first-message";
import type { PairWith } from "../pairing";
import { pausedNow } from "../pause";
import { Msg } from "../protocol";
import { badge } from "./badge";
import { adopted, framing } from "./held";
import { offer } from "./recovery";
import { request } from "./relay";
import { onServerMessage } from "./server-messages";
import { setSettings, settings } from "./state";
import { letGoAll } from "./tabs";
import { maybeReload } from "./update";
import { dialect, emit, link, serverJson, setLink, type Link } from "./wire";

const VERSION = chrome.runtime.getManifest().version;
const PONG = serverJson({ method: "pong" });

let pairing: PairWith | null = null;
let paired: PairWith | null = null;
let lastError = "";
let fails = 0;
let retryAt = 0;
let lastTick = Date.now();

/** Why the socket isn't up, for the popup. */
export function linkError(): string {
  return lastError;
}

/**
 * What the app shows about this profile: the extension's version, the Google account Chrome is signed in to (Chrome's
 * own answer; empty when it isn't), the OS and Chrome's version. Never anything from its pages, cookies or passwords.
 */
async function about(): Promise<About> {
  const email = await chrome.identity.getProfileUserInfo({ accountStatus: chrome.identity.AccountStatus.ANY }).then(
    (user) => user.email,
    () => undefined,
  );
  const os = await chrome.runtime.getPlatformInfo().then(
    (platform) => platform.os,
    () => undefined,
  );
  return { version: VERSION, email, os, agent: /Chrome\/[\d.]+/.exec(navigator.userAgent)?.[0] };
}

/** Connects when it should: paired (or pairing), no socket up, not backing off. A socket that's dead is let go. */
export async function connect(): Promise<void> {
  const s = await settings();
  const now = Date.now();
  const current = link();
  if (current && current.ws.readyState <= WebSocket.OPEN) {
    const open = current.ws.readyState === WebSocket.OPEN;
    if (!isStale({ open, since: current.since, answers: current.answers, missed: current.missed }, now)) return;
    current.ws.close();
    await gone(current, "The server stopped answering: connecting again");
  }
  if (!pairing && now < retryAt) return;
  const server = pairing?.server ?? s.server;
  if (!server || (!pairing && !s.token)) return;
  const asked = pairing;
  let ws: WebSocket;
  try {
    ws = new WebSocket(`${server.replace(/^http/, "ws")}/api/chrome/ws`);
  } catch {
    lastError = `Can't reach the server at ${server}`;
    return;
  }
  const made: Link = { ws, since: now, missed: 0, answers: false, ready: false };
  setLink(made);
  ws.onopen = () => inBackground(first(made, asked, s), "first message");
  ws.onmessage = (event: MessageEvent) => {
    made.missed = 0;
    if (typeof event.data !== "string") return;
    if (event.data === PONG) {
      made.answers = true;
      return;
    }
    inBackground(onMessage(made, server, event.data), "server message");
  };
  ws.onerror = () => {
    lastError = `Can't reach the server at ${server}`;
  };
  ws.onclose = (event: CloseEvent) => {
    if (asked && pairing === asked) pairing = null; // refused: pairWith ends with the server's words
    inBackground(gone(made, event.reason), "socket closed");
  };
}

/** The socket's first message: the pairing (its code only), or the hello (what this profile is). */
async function first(made: Link, asked: PairWith | null, saved: { server: string | null; token: string | null }): Promise<void> {
  const mine = asked ? { version: VERSION } : await about();
  made.ws.send(serverJson(firstMessage(asked, saved, mine)));
}

/** The socket closed: everything let go; connect again at once if it was in, else after the back-off. */
async function gone(made: Link, reason: string): Promise<void> {
  if (link() !== made) return;
  setLink(null);
  fails = made.ready ? 0 : fails + 1;
  retryAt = nextTry(fails, Date.now());
  if (reason) lastError = reason;
  await letGoAll("disconnected");
  adopted.clear();
  await badge();
  if (!fails) setTimeout(() => inBackground(connect(), "connect"), 300); // a tool may be waiting for it
}

/** A message on the socket `made`, opened for `server`; one from a socket that is no longer the current one is dropped. */
async function onMessage(made: Link, server: string, data: string): Promise<void> {
  if (link() !== made) return;
  let m: Message;
  try {
    const parsed: unknown = JSON.parse(data);
    if (typeof parsed !== "object" || parsed === null) return;
    m = readMessage(dialect, parsed as Message);
  } catch {
    return;
  }
  const params = (typeof m.params === "object" && m.params !== null ? m.params : {}) as Record<string, unknown>;
  if (m.method === Msg.paired) {
    // A pairing code worked: this profile's own token from now on, with the server it came from.
    const token = typeof params.token === "string" ? params.token : null;
    const name = typeof params.name === "string" ? params.name : null;
    const label = typeof params.label === "string" ? params.label : name;
    await setSettings({ token, name, label, server });
    paired = pairing;
    pairing = null;
    lastError = "";
    fails = retryAt = 0;
    setLink(null);
    made.ws.close(1000, "");
    return connect();
  }
  if (m.method === Msg.hello) {
    made.ready = true;
    lastError = "";
    if (typeof params.name === "string") {
      await setSettings({ name: params.name, label: typeof params.label === "string" ? params.label : params.name });
    }
    emit(Msg.stopped, pausedNow((await settings()).paused, Date.now()));
    await maybeReload();
    return badge();
  }
  if (!made.ready) return;
  if (m.id !== undefined) {
    try {
      made.ws.send(JSON.stringify({ id: m.id, result: await request(String(m.method), m.params ?? []) }));
    } catch (e) {
      made.ws.send(JSON.stringify({ id: m.id, error: messageOf(e) }));
    }
    return;
  }
  if (typeof m.method === "string") await onServerMessage(m.method, m.params);
}

/** Pairs with `pair`: this profile gets its own token on that server, replacing any earlier pairing. */
export async function pairWith(pair: PairWith): Promise<void> {
  pairing = pair;
  paired = null;
  lastError = "";
  const current = link();
  setLink(null);
  current?.ws.close(1000, "");
  await connect();
  for (let i = 0; i < 40 && pairing === pair; i++) await new Promise((done) => setTimeout(done, 250));
  if (paired === pair) return;
  if (pairing === pair) pairing = null;
  const left = link();
  if (left && !left.ready) {
    setLink(null);
    left.ws.close(1000, "");
  }
  throw new Error(lastError || "The server didn't answer");
}

/** Unpairs this profile: the server told, the token forgotten, the socket closed. */
export async function unpair(): Promise<void> {
  emit(Msg.unpair, {});
  await setSettings({ token: null, name: null, label: null });
  const current = link();
  setLink(null);
  current?.ws.close(1000, "");
}

/**
 * Every 20 s while the worker runs (which also keeps it alive while connected): a ping, a connect when there's none,
 * tabs another extension's frame keeps out offered again, and a waiting update tried. After a sleep the socket from
 * before it is dropped and the back-off forgotten.
 */
export function tick(): void {
  const now = Date.now();
  const sleptNow = slept(lastTick, now);
  lastTick = now;
  const current = link();
  if (sleptNow && current && fromBeforeSleep(current.since, now)) {
    current.ws.close();
    inBackground(gone(current, "The computer slept: connecting again"), "after sleep");
  }
  if (sleptNow) fails = retryAt = 0;
  const live = link();
  if (live?.ready) live.missed++;
  emit("ping", {});
  inBackground(connect(), "connect");
  for (const id of framing.keys()) inBackground(offer(id), "offer");
  inBackground(maybeReload(), "update");
}

/** The tick's cadence. */
export const TICK = TICK_MS;
