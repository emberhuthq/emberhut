import type { Instant } from "./common";

/** POST /api/auth/refresh, /api/auth/apple and /api/auth/pair. The refresh token turns over on every use. */
export interface Tokens {
  ok: boolean;
  status?: number;
  access?: string;
  refresh?: string;
  device?: number;
  expires?: Instant;
  error?: string;
  /**
   * POST /api/auth/pair: the machine's id and public key (docs/api.md, "Pairing: the machine and the device key"). Absent
   * from a server older than the field; read through checkMachine, never as it is.
   */
  machine?: unknown;
}

/** POST /api/auth/pair: a one-time code from Emberhut, and this install's identity. */
export interface PairRequest {
  code: string;
  install: string;
  name: string;
  model: string;
}

/** POST /api/auth/pair/new: a one-time code for another device, shown as ABCD-EFGH; it works once, until `expires`. */
export interface PairingCode {
  ok: boolean;
  code: string;
  expires: Instant;
  /** The pairing link with this code (`https://emberhut.com/pair#…`), shown as a QR code; absent from older servers. */
  link?: string;
}

/** Which of Apple's push services an iPhone's token belongs to: a development build's sandbox, or production. */
export type ApnsEnv = "sandbox" | "production";

/**
 * POST /api/devices: where Emberhut pushes this phone's alerts. An Android phone sends its FCM registration token with
 * `platform: "android"`; an iPhone sends its APNs device token (hex) and its service, and no platform.
 */
export type PushAddress = { token: string; platform: "android" } | { token: string; env: ApnsEnv };

export interface SignedInDevice {
  id: number;
  name?: string | null;
  model?: string | null;
  created?: Instant | null;
  seen?: Instant | null;
}

/** state.auth: report (requests without a session pass, logged) or enforce. */
export interface AuthInfo {
  mode: "report" | "enforce";
  bound: boolean;
  devices: SignedInDevice[];
}

/** GET /api/auth/status. */
export interface AuthStatus extends AuthInfo {
  /** This request came with a valid session. */
  signedIn: boolean;
}

/** A 401's body: `signIn` when the server enforces sign-in, otherwise the device is not on the server's network. */
export interface Unauthorized {
  error?: string;
  signIn?: boolean;
}
