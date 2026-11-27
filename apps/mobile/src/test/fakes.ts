import {
  AuthSession,
  createApi,
  EmberhutHttp,
  type Dialect,
  type Fetch,
  type SecureStorage,
  type ServerAddress,
  type UploadFile,
} from "@/api";

export interface Sent {
  url: URL;
  method: string;
  headers: Record<string, string>;
  body: unknown;
  signal?: AbortSignal;
}

type Handler = (request: Sent) => Response | Promise<Response>;

/** A stand-in server: each request is recorded, then answered by the first route whose method and path match. */
function fakeServer() {
  const sent: Sent[] = [];
  const routes: { method: string; path: string; handle: Handler }[] = [];
  const fetch: Fetch = async (url, init) => {
    const headers = Object.fromEntries(new Headers(init.headers).entries());
    const raw = init.body;
    const body = typeof raw === "string" && headers["content-type"] === "application/json" ? JSON.parse(raw) : raw;
    const request: Sent = { url: new URL(url), method: init.method ?? "GET", headers, body, signal: init.signal ?? undefined };
    sent.push(request);
    if (init.signal?.aborted) throw new DOMException("aborted", "AbortError");
    const route = routes.find((r) => r.method === request.method && r.path === request.url.pathname);
    if (!route) return json(404, { error: "not found" });
    return route.handle(request);
  };
  return {
    fetch,
    sent,
    on(method: string, path: string, handle: Handler) {
      routes.unshift({ method, path, handle });
    },
    to: (path: string) => sent.filter((r) => r.url.pathname === path),
  };
}

export function json(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(status === 304 ? null : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", ...headers },
  });
}

export function memoryStorage(initial: Record<string, string> = {}): SecureStorage & { items: Map<string, string> } {
  const items = new Map(Object.entries(initial));
  return {
    items,
    getItem: async (key) => items.get(key) ?? null,
    setItem: async (key, value) => void items.set(key, value),
    deleteItem: async (key) => void items.delete(key),
  };
}

/** Secure storage that fails while `failing` is set, as the Keychain does before the first unlock. */
export function flakyStorage(initial: Record<string, string> = {}) {
  const inner = memoryStorage(initial);
  const state = { failing: true };
  const check = () => {
    if (state.failing) throw new Error("User interaction is not allowed.");
  };
  const storage: SecureStorage & { items: Map<string, string> } = {
    items: inner.items,
    getItem: async (key) => (check(), inner.getItem(key)),
    setItem: async (key, value) => (check(), inner.setItem(key, value)),
    deleteItem: async (key) => (check(), inner.deleteItem(key)),
  };
  return { storage, state };
}

const baseUrl = "http://emberhut.test:18990";

/** The server address a paired test phone holds. */
export function fixedAddress(): ServerAddress {
  return {
    url: baseUrl,
    speaksDialect: true,
    load: async () => baseUrl,
    save: async () => undefined,
    setSpeaksDialect: async () => undefined,
    clear: async () => undefined,
  };
}

/** An API on a fake server, with a clock the test moves. */
export function fakeApi(options: { storage?: ReturnType<typeof memoryStorage>; dialect?: Dialect } = {}) {
  const server = fakeServer();
  const storage = options.storage ?? memoryStorage();
  const clock = { now: Date.parse("2026-11-24T10:00:00Z") };
  let ids = 0;
  const auth = new AuthSession({
    baseUrl: () => baseUrl,
    fetch: server.fetch,
    storage,
    newId: () => `install-${++ids}`,
    now: () => clock.now,
  });
  // A file "sent from disk" reaches the fake server as its URI.
  const uploadFile: UploadFile = (fileUri, url, { headers, signal }) =>
    server.fetch(url, { method: "POST", headers, body: fileUri, signal });
  const http = new EmberhutHttp({ baseUrl: () => baseUrl, fetch: server.fetch, auth, uploadFile, dialect: options.dialect });
  return { server, storage, clock, auth, http, api: createApi(http) };
}

/** A token answer like the server's, valid an hour from `now`. */
export function tokens(n: number, now: number) {
  return {
    ok: true,
    status: 200,
    access: `access-${n}`,
    refresh: `refresh-${n}`,
    device: 7,
    expires: new Date(now + 3_600_000).toISOString(),
  };
}

/** A body the test writes to as the stream goes. */
export function streamBody() {
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  const body = new ReadableStream<Uint8Array>({ start: (c) => void (controller = c) });
  const encoder = new TextEncoder();
  return {
    body,
    write: (text: string) => controller.enqueue(encoder.encode(text)),
    end: () => controller.close(),
  };
}
