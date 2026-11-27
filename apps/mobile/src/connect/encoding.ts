// The connect contract's encodings (docs/api.md, "Encodings"): binary values in JSON as base64url without padding, and
// machine ids as lowercase base32. Each value has one spelling, so a decoder refuses padding, other alphabets and
// non-zero trailing bits.

/** Why text isn't a value of the expected kind: the vectors' error names. */
export type DecodeError = "length" | "symbol" | "trailingBits";

export type Decoded = { ok: true; bytes: Uint8Array } | { ok: false; error: DecodeError };

const base64UrlAlphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const base32LowerAlphabet = "abcdefghijklmnopqrstuvwxyz234567";

/** Reads exactly `size` bytes of base64url without padding (RFC 4648, section 5). */
export function decodeBase64Url(text: string, size: number): Decoded {
  if (text.length !== Math.ceil((size * 8) / 6)) return { ok: false, error: "length" };
  const bytes = new Uint8Array(size);
  let buffer = 0;
  let bits = 0;
  let at = 0;
  for (const char of text) {
    const value = base64UrlAlphabet.indexOf(char);
    if (value < 0) return { ok: false, error: "symbol" };
    buffer = (buffer << 6) | value;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      if (at < size) bytes[at] = (buffer >> bits) & 0xff;
      at += 1;
      buffer &= (1 << bits) - 1;
    }
  }
  // What is left over is padding bits, and every one must be 0.
  if (buffer !== 0) return { ok: false, error: "trailingBits" };
  return { ok: true, bytes };
}

/** Lowercase base32 without padding (RFC 4648, section 6, in lower case). */
export function encodeBase32Lower(bytes: Uint8Array): string {
  let text = "";
  let buffer = 0;
  let bits = 0;
  for (const byte of bytes) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      text += base32LowerAlphabet[(buffer >> bits) & 31];
    }
    buffer &= (1 << bits) - 1;
  }
  if (bits > 0) text += base32LowerAlphabet[(buffer << (5 - bits)) & 31];
  return text;
}

/** Base64url without padding: the contract's one spelling of binary values in JSON. */
export function encodeBase64Url(bytes: Uint8Array): string {
  let text = "";
  let buffer = 0;
  let bits = 0;
  for (const byte of bytes) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 6) {
      bits -= 6;
      text += base64UrlAlphabet[(buffer >> bits) & 63];
    }
    buffer &= (1 << bits) - 1;
  }
  if (bits > 0) text += base64UrlAlphabet[(buffer << (6 - bits)) & 63];
  return text;
}
