// A wire dialect: for building the app for a server that spells some wire words differently. The build carries the
// dialect's JSON in EXPO_PUBLIC_EMBERHUT_WIRE_DIALECT; the API client reads every answer through it (into the app's own
// words, src/api/wire.ts) and writes every request through it (back into the server's). Unset or empty, nothing is
// rewritten. No imports, so app.config.ts reads it too.
//
// The JSON: { "words": [[server, app], ...], "keys": [...], "headers": [...], "prefixes": [...] }, every pair the server's
// spelling first. The rules are the same as the desktop client's (crates/emberhut-client/src/dialect.rs) and the browser
// extension's, and crates/emberhut-client/tests/fixtures/dialect.json holds the cases all three answer alike:
// - words: a whole string value that is exactly one of them, both ways. Reading, an object key that is one is renamed too
//   (after keys); writing, object keys and query names are renamed by keys only, since the app's words are field names
//   too ("owner");
// - keys: an object key or a query parameter's name (a field's name), both ways; reading, looked up before words;
// - headers: a header's name, in any case;
// - prefixes: text that starts a token inside a string value that isn't a whole word. A token starts at the start of the
//   string or right after a character that can't be part of one (token characters: letters, digits and _ - = + / : .),
//   so a token, an id or an address isn't rewritten inside; a link scheme (a prefix ending in ://) keeps the same rule.
//   A prefix ending in - matches only when a digit (0-9) follows it ("K-12", not "K-shirt"), either way. Where several
//   match at one place the longest wins (the first listed among equals). The text is read left to right as given: a
//   replacement is never searched again, and where a token starts is decided by the character before it in the given
//   text.
// Whole words, keys and headers: the first pair whose side matches wins.

/** The dialect's JSON, parsed: pairs of [the server's spelling, the app's]. */
export interface DialectTable {
  words: [string, string][];
  keys: [string, string][];
  headers: [string, string][];
  prefixes: [string, string][];
}

/** One direction's rewriting: from one side's spellings to the other's. */
interface Direction {
  /** From the server's spelling to the app's. */
  reading: boolean;
  words: Map<string, string>;
  keys: Map<string, string>;
  headers: Map<string, string>;
  /** The prefixes' [from, to] pairs, longest first (the first listed among equals). */
  prefixes: [string, string][];
}

/** Rewrites values between the server's spelling and the app's. */
export interface Dialect {
  /** No pair at all: every method gives back what it's given. */
  readonly identity: boolean;
  /** An answer's parsed JSON (a body, an event's data, a push's data), in the app's words. */
  read(value: unknown): unknown;
  /** A request's JSON, in the server's words. */
  write(value: unknown): unknown;
  /** One string from the server (a link, a line), in the app's words. */
  readText(text: string): string;
  /** One string for the server (a query value, a link), in its words. */
  writeText(text: string): string;
  /** A query parameter's name for the server (only `keys` rename one). */
  writeKey(key: string): string;
  /** A request header's name for the server. */
  writeHeader(name: string): string;
  /** An answer's header name, in the app's words. */
  readHeader(name: string): string;
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
      throw new Error(`The wire dialect's ${name} isn't a list of [server, app] pairs of two different non-empty strings`);
    }
    table[name as keyof DialectTable] = pairs.map(([server, app]) => [server, app]);
  }
  return table;
}

/** A character that can be part of a token: a letter, a digit or one of _ - = + / : . */
const TOKEN = /^[\p{Alphabetic}\p{N}_\-=+/:.]$/u;
const DIGIT = /^[0-9]$/;

function direction(pairs: DialectTable, from: 0 | 1): Direction {
  const to = from === 0 ? 1 : 0;
  const map = (list: [string, string][], fold = (s: string) => s) => {
    const out = new Map<string, string>();
    for (const pair of list) if (!out.has(fold(pair[from]))) out.set(fold(pair[from]), pair[to]);
    return out;
  };
  // A stable sort: among prefixes of one length, the first listed comes first.
  const prefixes = pairs.prefixes.map((pair): [string, string] => [pair[from], pair[to]]).sort((a, b) => b[0].length - a[0].length);
  return {
    reading: from === 0,
    words: map(pairs.words),
    keys: map(pairs.keys),
    headers: map(pairs.headers, (s) => s.toLowerCase()),
    prefixes,
  };
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

/** An object key: renamed by `keys`, and by `words` too when reading (see the top). */
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
  const header = (way: Direction, name: string) => way.headers.get(name.toLowerCase()) ?? name;
  return {
    identity: false,
    read: (value) => deep(reading, value),
    write: (value) => deep(writing, value),
    readText: (value) => text(reading, value),
    writeText: (value) => text(writing, value),
    writeKey: (name) => key(writing, name),
    writeHeader: (name) => header(writing, name),
    readHeader: (name) => header(reading, name),
  };
}

const same = <T>(value: T): T => value;

/** The app's own words, unchanged both ways. */
export const identityDialect: Dialect = {
  identity: true,
  read: same,
  write: same,
  readText: same,
  writeText: same,
  writeKey: same,
  writeHeader: same,
  readHeader: same,
};

/**
 * This build's dialect. Read as `process.env.EXPO_PUBLIC_…` literally, which Metro inlines when it bundles; app.config.ts
 * reads the same variable from the environment.
 */
export const dialect: Dialect = createDialect(parseDialect(process.env.EXPO_PUBLIC_EMBERHUT_WIRE_DIALECT));

/** A JSON document taken from the server some way other than through the API client, in the app's words (this build's dialect). */
export const jsonFromServer = (value: unknown): unknown => dialect.read(value);

/** A JSON document for the server sent some way other than through the API client, in its words (this build's dialect). */
export const jsonToServer = (value: unknown): unknown => dialect.write(value);

/** One string from the server (a link, a line), in the app's words (this build's dialect). */
export const fromServer = (text: string): string => dialect.readText(text);

/** One string for the server (a link, a value), in its words (this build's dialect). */
export const toServer = (text: string): string => dialect.writeText(text);
