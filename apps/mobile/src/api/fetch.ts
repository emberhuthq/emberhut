/** The one fetch the API layer uses: expo/fetch in the app (it streams response bodies), a stand-in in tests. */
export type Fetch = (url: string, init: RequestInit) => Promise<Response>;

/**
 * Sends a file on the device as a POST's whole body, read from disk as it goes (expo/fetch reads a Blob body into memory
 * first): expo-file-system's native upload in the app, a stand-in in tests.
 */
export type UploadFile = (
  fileUri: string,
  url: string,
  init: { headers: Record<string, string>; signal?: AbortSignal },
) => Promise<Response>;

/**
 * A path on Emberhut's own server ("/api/…", "/uploads/…"): never "//host/…" or "/\\host/…", which a URL reads as another
 * server, and no tab or line break anywhere, which a URL drops ("/\t/host" is "//host"). buildUrl refuses the same.
 */
export const isServerPath = (path: string): boolean => /^\/(?![/\\])/.test(path) && !/[\t\n\r]/.test(path);

/**
 * Joins a server path and query onto the base URL. Undefined and null values are left out. Anything that would leave
 * the server (another origin, "//host", "/\\host") is refused: the session rides with every request built here.
 */
export function buildUrl(baseUrl: string, path: string, query?: Query): string {
  const base = new URL(baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`);
  const url = new URL(path, base);
  if (!isServerPath(path) || url.origin !== base.origin) throw new Error(`Not a path on Emberhut's server: ${path}`);
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value !== undefined && value !== null) url.searchParams.set(key, String(value));
  }
  return url.toString();
}

export type Query = Record<string, string | number | boolean | null | undefined>;

/** A signal that aborts after `ms`, or when `outer` does. `done` clears the timer. */
export function timeoutSignal(ms: number, outer?: AbortSignal): { signal: AbortSignal; done: () => void } {
  const controller = new AbortController();
  const abort = () => controller.abort();
  const timer = setTimeout(abort, ms);
  if (outer?.aborted) controller.abort();
  outer?.addEventListener("abort", abort);
  return {
    signal: controller.signal,
    done: () => {
      clearTimeout(timer);
      outer?.removeEventListener("abort", abort);
    },
  };
}

export async function readJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (text === "") return undefined;
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}
