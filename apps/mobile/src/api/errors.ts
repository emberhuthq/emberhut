/** A request that failed: `status` 0 means the server was never reached (network, timeout). */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
    readonly body?: unknown,
  ) {
    super(message);
    this.name = "ApiError";
  }

  get unreachable(): boolean {
    return this.status === 0;
  }
}

/**
 * This phone's secure storage (Keychain, Android Keystore) couldn't be read or written: before the first unlock after a
 * restart, or an entry the Keystore can no longer decrypt after a restore. Nothing was cached: the next call reads again.
 */
export class StorageError extends Error {
  constructor(message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.name = "StorageError";
  }
}

/** The server's reason in an error body (`{ error }`), if it gave one. */
export function reasonOf(body: unknown): string | undefined {
  if (typeof body !== "object" || body === null || !("error" in body)) return undefined;
  const error = (body as { error: unknown }).error;
  return typeof error === "string" && error !== "" ? error : undefined;
}
