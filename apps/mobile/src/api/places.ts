import type { AuthSession } from "./auth";
import { contractWords, type Dialect } from "./dialect";
import type { Fetch, UploadFile } from "./fetch";
import { EmberhutHttp } from "./http";
import type { ServerAddress, Slot } from "./server";

/** One server's place on this phone: its address and its session. */
export interface ServerPlace {
  address: ServerAddress;
  auth: AuthSession;
}

export interface ServerHttpOptions {
  places: Record<Slot, ServerPlace>;
  /** The server requests go to now. */
  using: () => Slot;
  fetch: Fetch;
  uploadFile?: UploadFile;
  /** This build's dialect, spoken only to a server marked as speaking it. */
  dialect: Dialect;
}

/** The reserved .invalid name: a request before any server was kept fails at once. */
const nowhere = "http://server.invalid";

/** The words to speak to a server: this build's dialect if it speaks it, the contract's otherwise (task keys in the app's prefix). */
const wordsFor = (address: ServerAddress, dialect: Dialect): Dialect => (address.speaksDialect ? dialect : contractWords);

/**
 * The HTTP clients: `inUse` sends to whichever server `using` names at each request (its address, its session, its
 * words), and `own` to one server whatever is in use (to revoke this phone's device there).
 */
export function createServerHttp({ places, using, fetch, uploadFile, dialect }: ServerHttpOptions) {
  const place = () => places[using()];
  const inUse = new EmberhutHttp({
    baseUrl: () => place().address.url ?? nowhere,
    fetch,
    auth: {
      accessToken: () => place().auth.accessToken(),
      currentAccess: () => place().auth.currentAccess(),
      noteResponse: (status, body) => place().auth.noteResponse(status, body),
    },
    uploadFile,
    dialect: () => wordsFor(place().address, dialect),
  });
  const own = (slot: Slot) =>
    new EmberhutHttp({
      baseUrl: () => places[slot].address.url ?? nowhere,
      fetch,
      auth: places[slot].auth,
      uploadFile,
      dialect: () => wordsFor(places[slot].address, dialect),
    });
  return { inUse, own: { server: own("server"), fallback: own("fallback") } };
}
