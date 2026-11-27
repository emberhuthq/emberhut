// application/x-www-form-urlencoded, read as the WHATWG URL Standard's parser (and Rust's form_urlencoded, which the
// server's link writer pairs with) reads it. Written out rather than taken from URLSearchParams, which React Native
// doesn't fully implement.

/** The name and value pairs of a form-encoded string, in order; repeated names stay repeated. */
export function parseForm(input: string): [string, string][] {
  const pairs: [string, string][] = [];
  for (const sequence of input.split("&")) {
    if (sequence === "") continue;
    const equals = sequence.indexOf("=");
    const name = equals < 0 ? sequence : sequence.slice(0, equals);
    const value = equals < 0 ? "" : sequence.slice(equals + 1);
    pairs.push([decodeComponent(name), decodeComponent(value)]);
  }
  return pairs;
}

const encoder = new TextEncoder();

/** `+` is a space, `%XX` a byte (a `%` not followed by two hex digits stays as it is), and the bytes are UTF-8. */
function decodeComponent(text: string): string {
  const bytes = encoder.encode(text.replace(/\+/g, " "));
  const out: number[] = [];
  for (let i = 0; i < bytes.length; i += 1) {
    const byte = bytes[i]!;
    if (byte === 0x25 && i + 2 < bytes.length && isHex(bytes[i + 1]!) && isHex(bytes[i + 2]!)) {
      out.push(parseInt(String.fromCharCode(bytes[i + 1]!, bytes[i + 2]!), 16));
      i += 2;
    } else {
      out.push(byte);
    }
  }
  return decodeUtf8Lossy(out);
}

const isHex = (byte: number): boolean => (byte >= 0x30 && byte <= 0x39) || (byte >= 0x41 && byte <= 0x46) || (byte >= 0x61 && byte <= 0x66);

/**
 * UTF-8 to text, each ill-formed sequence replaced by U+FFFD as the Encoding Standard's decoder does it (one per maximal
 * subpart, the rule Rust's from_utf8_lossy follows too).
 */
export function decodeUtf8Lossy(bytes: readonly number[]): string {
  let text = "";
  let i = 0;
  while (i < bytes.length) {
    const first = bytes[i]!;
    if (first < 0x80) {
      text += String.fromCharCode(first);
      i += 1;
      continue;
    }
    // The sequence's length and the range its second byte must fall in (which rules out overlong forms and surrogates).
    let needed: number;
    let low = 0x80;
    let high = 0xbf;
    let point: number;
    if (first >= 0xc2 && first <= 0xdf) {
      needed = 1;
      point = first & 0x1f;
    } else if (first >= 0xe0 && first <= 0xef) {
      needed = 2;
      point = first & 0x0f;
      if (first === 0xe0) low = 0xa0;
      if (first === 0xed) high = 0x9f;
    } else if (first >= 0xf0 && first <= 0xf4) {
      needed = 3;
      point = first & 0x07;
      if (first === 0xf0) low = 0x90;
      if (first === 0xf4) high = 0x8f;
    } else {
      text += "�";
      i += 1;
      continue;
    }
    let j = i + 1;
    let complete = true;
    for (let k = 0; k < needed; k += 1, j += 1) {
      const next = bytes[j];
      if (next === undefined || next < low || next > high) {
        complete = false;
        break;
      }
      point = (point << 6) | (next & 0x3f);
      low = 0x80;
      high = 0xbf;
    }
    if (complete) {
      text += String.fromCodePoint(point);
      i = j;
    } else {
      // The bytes read so far are one maximal subpart; the byte that broke it starts again.
      text += "�";
      i = j;
    }
  }
  return text;
}
