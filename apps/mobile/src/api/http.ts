import type { AuthSession } from "./auth";
import { dialect, type Dialect } from "./dialect";
import { ApiError, reasonOf } from "./errors";
import { buildUrl, readJson, timeoutSignal, type Fetch, type Query, type UploadFile } from "./fetch";

export interface RequestOptions {
  /** GET without a body, POST with one, unless given. */
  method?: "GET" | "POST" | "PUT" | "DELETE";
  query?: Query;
  /** A JSON body; makes the request a POST. */
  json?: unknown;
  /** A raw body (a file's bytes); makes the request a POST. */
  body?: BodyInit;
  /** A file on the device (its file:// URI) as the body, sent from disk through `uploadFile`; makes the request a POST. */
  file?: string;
  contentType?: string;
  headers?: Record<string, string>;
  /** 15 s by default; 0 for none (a stream). */
  timeoutMs?: number;
  signal?: AbortSignal;
}

/** What requests need of a session: an AuthSession, or one standing for whichever server is in use. */
export type RequestAuth = Pick<AuthSession, "accessToken" | "currentAccess" | "noteResponse">;

export interface EmberhutHttpOptions {
  /** The server, as it is now: it's typed in at pairing, and may be the fallback server while the server is down. */
  baseUrl: () => string;
  fetch: Fetch;
  auth: RequestAuth;
  uploadFile?: UploadFile;
  /**
   * How the server spells the wire words: this build's dialect by default (src/api/dialect.ts). A function is asked at
   * each request: the server in use may change, and only one that speaks the dialect is spoken to in it.
   */
  dialect?: Dialect | (() => Dialect);
}

/** Requests to Emberhut's server with this device's session. */
export class EmberhutHttp {
  constructor(private readonly options: EmberhutHttpOptions) {}

  get baseUrl(): string {
    return this.options.baseUrl();
  }

  /** The wire words' spelling on this server: answers are read through it, requests written through it. */
  get dialect(): Dialect {
    const given = this.options.dialect;
    return typeof given === "function" ? given() : (given ?? dialect);
  }

  url(path: string, query?: Query): string {
    return buildUrl(this.options.baseUrl(), path, query && this.serverQuery(query));
  }

  /** An answer's JSON body, in the app's words; undefined when it has none or isn't JSON. */
  async readBody(response: Response): Promise<unknown> {
    return this.dialect.read(await readJson(response));
  }

  private serverQuery(query: Query): Query {
    const { dialect: words } = this;
    if (words.identity) return query;
    return Object.fromEntries(
      Object.entries(query).map(([key, value]) => [words.writeKey(key), typeof value === "string" ? words.writeText(value) : value]),
    );
  }

  /** A server file's URL for a player that can't send headers: the session rides in the address. */
  fileUrl(path: string): string {
    // The token is opaque: it goes as it is, past the dialect.
    return buildUrl(this.options.baseUrl(), path, { token: this.options.auth.currentAccess() });
  }

  /** A server file as an image source, the session in its header. */
  fileSource(path: string): { uri: string; headers: Record<string, string> } {
    const access = this.options.auth.currentAccess();
    return { uri: this.url(path), headers: access ? { Authorization: `Bearer ${access}` } : {} };
  }

  /** The raw answer for any status but 401; throws ApiError(0) when the server can't be reached. */
  async send(path: string, options: RequestOptions = {}): Promise<Response> {
    const { auth, fetch, uploadFile } = this.options;
    const url = this.url(path, options.query);
    if (options.file !== undefined && !uploadFile) throw new Error("This API has no uploadFile to send a file with");
    const headers: Record<string, string> = {};
    for (const [name, value] of Object.entries(options.headers ?? {})) headers[this.dialect.writeHeader(name)] = value;
    const access = await auth.accessToken();
    if (access !== null) headers.Authorization = `Bearer ${access}`;
    let body: BodyInit | undefined = options.body;
    if (options.json !== undefined) {
      body = JSON.stringify(this.dialect.write(options.json));
      headers["Content-Type"] = "application/json";
    } else if (body !== undefined || options.file !== undefined) {
      headers["Content-Type"] = options.contentType ?? "application/octet-stream";
    }
    const method = options.method ?? (body === undefined && options.file === undefined ? "GET" : "POST");
    const timeout = options.timeoutMs ?? 15_000;
    const limit = timeout > 0 ? timeoutSignal(timeout, options.signal) : undefined;
    const signal = limit?.signal ?? options.signal;
    let response: Response;
    try {
      response =
        options.file !== undefined && uploadFile
          ? await uploadFile(options.file, url, { headers, signal })
          : await fetch(url, { method, headers, body, signal });
    } catch (cause) {
      if (options.signal?.aborted) throw cause;
      throw new ApiError(0, "Can't reach Emberhut");
    } finally {
      limit?.done(); // the timeout covers reaching the server and its headers
    }
    if (response.status === 401) {
      const unauthorized = await this.readBody(response);
      auth.noteResponse(401, unauthorized);
      throw new ApiError(401, reasonOf(unauthorized) ?? "Not signed in", unauthorized);
    }
    auth.noteResponse(response.status, undefined);
    return response;
  }

  /** A JSON answer; throws ApiError on an error status or an `{ ok: false }` body. */
  async json<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const response = await this.send(path, options);
    const body = await this.readBody(response);
    if (!response.ok) throw new ApiError(response.status, reasonOf(body) ?? `HTTP ${response.status}`, body);
    if (typeof body === "object" && body !== null && (body as { ok?: unknown }).ok === false) {
      throw new ApiError(response.status, reasonOf(body) ?? "Refused", body);
    }
    return body as T;
  }

  get<T>(path: string, query?: Query, options: Omit<RequestOptions, "query"> = {}): Promise<T> {
    return this.json<T>(path, { ...options, query });
  }

  post<T>(path: string, json: unknown = {}, options: Omit<RequestOptions, "json"> = {}): Promise<T> {
    return this.json<T>(path, { ...options, json });
  }

  put<T>(path: string, json: unknown, options: Omit<RequestOptions, "json" | "method"> = {}): Promise<T> {
    return this.json<T>(path, { ...options, json, method: "PUT" });
  }

  delete<T>(path: string, options: Omit<RequestOptions, "method"> = {}): Promise<T> {
    return this.json<T>(path, { ...options, method: "DELETE" });
  }
}
