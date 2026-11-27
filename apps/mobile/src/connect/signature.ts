import { ed25519 } from "@noble/curves/ed25519.js";
import { bytesToNumberLE, concatBytes, equalBytes } from "@noble/curves/utils.js";
import { sha512 } from "@noble/hashes/sha2.js";

// Ed25519 by the contract's one verification rule (docs/api.md, "Statements"), so whether a signature verifies never
// depends on the library. A signature R || s by key A over M verifies when s < L; A and R are canonical encodings of
// points in the prime-order subgroup, neither of small order; and [s]B = R + [k]A without the cofactor, with
// k = SHA-512(R || A || M) mod L. noble's default is the cofactored ZIP 215 check, which accepts what this refuses, so
// the rule is written out here on its points, read with `zip215` off (canonical encodings only).

const Point = ed25519.Point;
type EdPoint = ReturnType<typeof Point.fromBytes>;
/** The prime order of the group B generates. */
const order = Point.Fn.ORDER;

/** A canonical encoding of a point in the prime-order subgroup, not of small order; null for anything else. */
function readPoint(bytes: Uint8Array): EdPoint | null {
  if (bytes.length !== 32) return null;
  let point: EdPoint;
  try {
    point = Point.fromBytes(bytes, false);
  } catch {
    return null;
  }
  return point.isSmallOrder() || !point.isTorsionFree() ? null : point;
}

/** Whether 32 bytes are a public key the rule accepts at all: a key that fails it doesn't read. */
export const isPublicKey = (bytes: Uint8Array): boolean => readPoint(bytes) !== null;

/** Whether `signature` (64 bytes) by `publicKey` (32 bytes) verifies over `message`, by the contract's rule. */
export function verifyStrict(publicKey: Uint8Array, message: Uint8Array, signature: Uint8Array): boolean {
  if (signature.length !== 64) return false;
  const a = readPoint(publicKey);
  const rBytes = signature.subarray(0, 32);
  const r = readPoint(rBytes);
  if (a === null || r === null) return false;
  const s = bytesToNumberLE(signature.subarray(32));
  if (s >= order) return false;
  const k = bytesToNumberLE(sha512(concatBytes(rBytes, publicKey, message))) % order;
  // multiplyUnsafe takes 0 too (s or k may be): these are public values, so constant time isn't needed.
  const left = Point.BASE.multiplyUnsafe(s);
  const right = r.add(a.multiplyUnsafe(k));
  return equalBytes(left.toBytes(), right.toBytes());
}
