import { sha256 } from "@noble/hashes/sha2.js";
import { decodeBase64Url, encodeBase32Lower, type DecodeError } from "./encoding";
import { isPublicKey } from "./signature";

// A machine's id is a name derived from its public key (docs/api.md, "Encodings"): the first 160 bits of
// SHA-256(statement("emberhut machine id v1", [public key])), in lowercase base32 without padding. Whoever shows the key proves
// the id, so a phone that knows an id can check that a key belongs to it. Hermes has no WebCrypto, so the hash is
// @noble/hashes, the same code in the app and in the tests.

/** An Ed25519 public key's size in bytes. */
const keyBytes = 32;
/** The id's size: 160 bits. */
const idBytes = 20;
/** The id as text: 32 characters from a-z and 2-7. */
export const machineIdLength = 32;

const encoder = new TextEncoder();

/**
 * statement(prefix, fields): the ASCII prefix, a 0 byte, then each field as a 4-byte big-endian length and its bytes.
 * Every signed statement and the id's hash input share it.
 */
export function statement(prefix: string, fields: readonly Uint8Array[]): Uint8Array {
  const head = encoder.encode(prefix);
  const out = new Uint8Array(head.length + 1 + fields.reduce((sum, field) => sum + 4 + field.length, 0));
  out.set(head);
  let at = head.length + 1;
  for (const field of fields) {
    new DataView(out.buffer).setUint32(at, field.length);
    out.set(field, at + 4);
    at += 4 + field.length;
  }
  return out;
}

/** The id of a public key's 32 bytes. */
export function machineIdOf(publicKey: Uint8Array): string {
  return encodeBase32Lower(sha256(statement("emberhut machine id v1", [publicKey])).subarray(0, idBytes));
}

/**
 * Why text isn't a machine id, or null when it is one (exactly 32 characters of a-z and 2-7). Checked in the Rust
 * reader's order: upper case anywhere first, then the length in bytes, then the alphabet.
 */
export function machineIdError(text: string): DecodeError | null {
  if (/[A-Z]/.test(text)) return "symbol";
  if (encoder.encode(text).length !== machineIdLength) return "length";
  return /^[a-z2-7]+$/.test(text) ? null : "symbol";
}

/** A machine's id and public key (base64url), as a pairing answer or the account's list carries them. */
export interface MachineKey {
  id: string;
  publicKey: string;
}

/** What a pairing answer's `machine` proved about the server that sent it. */
export type MachineCheck =
  /** It showed a key that has its id, and the id is the one expected (when one was). */
  | { ok: true; machine: MachineKey | null }
  /** No key, or the wrong one: "This server didn't prove it is the machine you meant". */
  | { ok: false; reason: "missing" | "wrongKey" | "otherMachine" };

/**
 * Checks a pairing answer's `machine` (docs/api.md, "Pairing: the machine and the device key"). Its key must give its id.
 * When the phone expects an id (it pairs from a link, a machine picked from the account, or a host it already holds a
 * key for), the answer must carry `machine`, with that id: stripping the field can't downgrade the check. Only a first,
 * typed pairing (`expected` null) takes an answer without one, from a server older than the field.
 *
 * The key must be a public key the contract's signature rule accepts (a canonical point of the prime-order subgroup, not
 * of small order), so a pinned key can verify the machine's signatures later.
 */
export function checkMachine(machine: unknown, expected: string | null): MachineCheck {
  if (machine === undefined || machine === null) return expected === null ? { ok: true, machine: null } : { ok: false, reason: "missing" };
  if (typeof machine !== "object") return { ok: false, reason: "wrongKey" };
  const { id, publicKey } = machine as Record<string, unknown>;
  if (typeof id !== "string" || typeof publicKey !== "string") return { ok: false, reason: "wrongKey" };
  const key = decodeBase64Url(publicKey, keyBytes);
  if (!key.ok || !isPublicKey(key.bytes) || machineIdOf(key.bytes) !== id) return { ok: false, reason: "wrongKey" };
  if (expected !== null && id !== expected) return { ok: false, reason: "otherMachine" };
  return { ok: true, machine: { id, publicKey } };
}

/** What the phone says when a pairing answer fails checkMachine. */
export const notTheMachine = "This server didn't prove it is the machine you meant.";
