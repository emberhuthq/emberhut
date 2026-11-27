// A wire dialect: for building the extension for a server that spells some of the protocol's words differently. The
// build reads the dialect file named by EMBERHUT_WIRE_DIALECT (scripts/build.ts) and injects its JSON; unset, nothing is
// rewritten. No imports, so the build script loads this file as it is to check the file.
//
// The file is the one the desktop client and the mobile app are built with, read by the same rules
// (crates/emberhut-client/src/dialect.rs, apps/mobile/README.md; crates/emberhut-client/tests/fixtures/dialect.json holds
// the cases all three answer alike): { "words": [[server, ours], ...], "keys": [...], "headers": [...], "prefixes": [...] },
// every pair the server's spelling first.
// - words: a whole string value that is exactly one of them, both ways; reading, an object key that is one too (after keys);
// - keys: an object key, both ways; reading, looked up before words; writing, the only way a key is renamed;
// - headers: accepted, unused (the extension sends no header of the protocol's own);
// - prefixes: text that starts a token inside a string value that isn't a whole word. A token starts at the start of the
//   string or right after a character that can't be part of one (token characters: letters, digits and _ - = + / : .); a
//   link scheme (a prefix ending in ://) keeps the same rule. A prefix ending in - matches only when a digit (0-9) follows
//   it, either way. Where several match at one place the longest wins (the first listed among equals). The text is read
//   left to right as given: a replacement is never searched again, and where a token starts is decided by the character
//   before it in the given text.
// Whole words and keys: the first pair whose side matches wins.

/** The dialect's JSON, parsed: pairs of [the server's spelling, the extension's]. */
export interface DialectTable {
  words: [string, string][];
  keys: [string, string][];
  headers: [string, string][];
  prefixes: [string, string][];
}

/** Rewrites values between the server's spelling and the extension's. */
export interface Dialect {
  /** No pair at all: every method gives back what it's given. */
  readonly identity: boolean;
  /** One string from the server, in the extension's words. */
  fromServer(text: string): string;
  /** One string for the server, in its words. */
  toServer(text: string): string;
  /** A JSON value from the server, in the extension's words. */
  jsonFromServer(value: unknown): unknown;
  /** A JSON value for the server, in its words. */
  jsonToServer(value: unknown): unknown;
}

const sections = ["words", "keys", "headers", "prefixes"] as const;

/** The dialect's JSON as a table; unset or blank is the empty one. Anything else malformed throws, naming what. */
export function parseDialect(json: string | undefined): DialectTable {
  const table: DialectTable = { words: [], keys: [], headers: [], prefixes: [] };
  if (json === undefined || json.trim() === "") return table;
  let value: unknown;
  try {
    value = JSON.parse(json);
  } catch {
    throw new Error("The wire dialect isn't JSON");
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("The wire dialect isn't a JSON object");
  for (const [name, pairs] of Object.entries(value)) {
    if (!(sections as readonly string[]).includes(name)) throw new Error(`The wire dialect has an unknown section: ${name}`);
    const isPair = (pair: unknown): pair is [string, string] =>
      Array.isArray(pair) && pair.length === 2 && pair.every((side) => typeof side === "string" && side !== "") && pair[0] !== pair[1];
    if (!Array.isArray(pairs) || !pairs.every(isPair)) {
      throw new Error(`The wire dialect's ${name} isn't a list of [server, extension] pairs of two different non-empty strings`);
    }
    table[name as keyof DialectTable] = pairs.map(([server, ours]) => [server, ours]);
  }
  return table;
}

/** A character that can be part of a token: a letter, a digit or one of _ - = + / : . */
const TOKEN = /^[\p{Alphabetic}\p{N}_\-=+/:.]$/u;
const DIGIT = /^[0-9]$/;

/** One direction's rewriting: from one side's spellings to the other's. */
interface Direction {
  /** From the server's spelling to the extension's. */
  reading: boolean;
  words: Map<string, string>;
  keys: Map<string, string>;
  /** The prefixes' [from, to] pairs, longest first (the first listed among equals). */
  prefixes: [string, string][];
}

function direction(table: DialectTable, from: 0 | 1): Direction {
  const to = from === 0 ? 1 : 0;
  const map = (list: [string, string][]) => {
    const out = new Map<string, string>();
    for (const pair of list) if (!out.has(pair[from])) out.set(pair[from], pair[to]);
    return out;
  };
  // A stable sort: among prefixes of one length, the first listed comes first.
  const prefixes = table.prefixes.map((pair): [string, string] => [pair[from], pair[to]]).sort((a, b) => b[0].length - a[0].length);
  return { reading: from === 0, words: map(table.words), keys: map(table.keys), prefixes };
}

function text(way: Direction, value: string): string {
  const word = way.words.get(value);
  if (word !== undefined) return word;
  if (way.prefixes.length === 0) return value;
  let out = "";
  let copied = 0;
  let at = 0;
  let before = "";
  while (at < value.length) {
    const found = TOKEN.test(before) ? undefined : prefixAt(way, value, at);
    if (found !== undefined) {
      out += value.slice(copied, at) + found[1];
      at += found[0].length;
      copied = at;
      before = [...found[0]].pop()!;
    } else {
      before = String.fromCodePoint(value.codePointAt(at)!);
      at += before.length;
    }
  }
  return copied === 0 ? value : out + value.slice(copied);
}

/** The prefix pair whose side `value` has at `at`: the longest (they're sorted so); one ending in - only before a digit. */
function prefixAt(way: Direction, value: string, at: number): [string, string] | undefined {
  return way.prefixes.find(([from]) => value.startsWith(from, at) && (!from.endsWith("-") || DIGIT.test(value.charAt(at + from.length))));
}

/** An object key: renamed by `keys`, and by `words` too when reading. */
function key(way: Direction, name: string): string {
  return way.keys.get(name) ?? (way.reading ? way.words.get(name) : undefined) ?? name;
}

function deep(way: Direction, value: unknown): unknown {
  if (typeof value === "string") return text(way, value);
  if (Array.isArray(value)) return value.map((item) => deep(way, item));
  if (typeof value === "object" && value !== null) {
    const out: Record<string, unknown> = {};
    for (const [name, item] of Object.entries(value)) out[key(way, name)] = deep(way, item);
    return out;
  }
  return value;
}

/** A dialect from its table. */
export function createDialect(table: DialectTable): Dialect {
  if (sections.every((name) => table[name].length === 0)) return identityDialect;
  const reading = direction(table, 0);
  const writing = direction(table, 1);
  return {
    identity: false,
    fromServer: (value) => text(reading, value),
    toServer: (value) => text(writing, value),
    jsonFromServer: (value) => deep(reading, value),
    jsonToServer: (value) => deep(writing, value),
  };
}

const same = <T>(value: T): T => value;

/** The extension's own words, unchanged both ways. */
export const identityDialect: Dialect = {
  identity: true,
  fromServer: same,
  toServer: same,
  jsonFromServer: same,
  jsonToServer: same,
};
