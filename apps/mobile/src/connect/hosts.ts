// Hosts and socket addresses in the connect contract's normal form (docs/api.md, "Encodings"), as far as the phone needs
// them: to check that a probe's answer is in normal form, and to compare it with the route it was sent on. A probe's
// `host` and `local` are compared as text, so the route's side is put in the same form here.

const ipv4Pattern = /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/;

/** An IPv4 address in dotted decimal, each part without leading zeros. */
export const isIpv4 = (text: string): boolean => ipv4Pattern.test(text);

/** An IPv6 address's eight 16-bit groups, or null when the text isn't one (no zone id, an IPv4 tail allowed). */
export function parseIpv6(text: string): number[] | null {
  if (!/^[0-9a-fA-F:.]+$/.test(text)) return null;
  let body = text;
  const tail: number[] = [];
  if (text.includes(".")) {
    const cut = text.lastIndexOf(":");
    const ipv4 = text.slice(cut + 1);
    if (cut < 0 || !isIpv4(ipv4)) return null;
    const [a, b, c, d] = ipv4.split(".").map(Number) as [number, number, number, number];
    tail.push((a << 8) | b, (c << 8) | d);
    body = text.slice(0, cut + 1);
    if (!body.endsWith("::")) body = body.slice(0, -1);
  }
  const halves = body.split("::");
  if (halves.length > 2) return null;
  const groups = (half: string): number[] | null => {
    if (half === "") return [];
    const parts = half.split(":");
    return parts.every((part) => /^[0-9a-fA-F]{1,4}$/.test(part)) ? parts.map((part) => parseInt(part, 16)) : null;
  };
  const head = groups(halves[0]!);
  const rest = halves.length === 2 ? groups(halves[1]!) : [];
  if (head === null || rest === null) return null;
  const given = head.length + rest.length + tail.length;
  if (halves.length === 1) return given === 8 ? [...head, ...tail] : null;
  if (given > 7) return null;
  return [...head, ...new Array<number>(8 - given).fill(0), ...rest, ...tail];
}

/** RFC 5952's text form: lowercase, no leading zeros, the longest run of two or more zero groups (the first) as `::`. */
export function formatIpv6(groups: readonly number[]): string {
  let bestStart = -1;
  let bestLength = 1;
  for (let i = 0; i < groups.length;) {
    if (groups[i] !== 0) {
      i += 1;
      continue;
    }
    let j = i;
    while (j < groups.length && groups[j] === 0) j += 1;
    if (j - i > bestLength) {
      bestStart = i;
      bestLength = j - i;
    }
    i = j;
  }
  const text = (part: readonly number[]) => part.map((group) => group.toString(16)).join(":");
  if (bestStart < 0) return text(groups);
  return `${text(groups.slice(0, bestStart))}::${text(groups.slice(bestStart + bestLength))}`;
}

/** `::ffff:a.b.c.d`: the IPv4 address names that host, so the contract never writes it this way. */
const isIpv4Mapped = (groups: readonly number[]): boolean => groups.slice(0, 5).every((group) => group === 0) && groups[5] === 0xffff;

/** An IPv6 literal (without brackets) already in RFC 5952 form, and not IPv4-mapped. */
function isNormalIpv6(text: string): boolean {
  const groups = parseIpv6(text);
  return groups !== null && !isIpv4Mapped(groups) && formatIpv6(groups) === text;
}

/** A port as the normal form writes it: 1 to 65535, no leading zero or sign. */
const isNormalPort = (text: string): boolean => /^[1-9]\d{0,4}$/.test(text) && Number(text) <= 65_535;

/**
 * A DNS name in normal form: lowercase ASCII letters, digits, hyphens and underscores, labels of 1 to 63 characters with
 * no hyphen at either end nor in the third and fourth places (but `xn--`), at most 253 characters, no trailing dot; and
 * one whose last label is a number must be an IPv4 address. (An `xn--` label is taken as written: the phone never
 * makes one, it only compares.)
 */
function isNormalName(text: string): boolean {
  if (text.length === 0 || text.length > 253) return false;
  const labels = text.split(".");
  if (/^\d+$/.test(labels[labels.length - 1]!)) return isIpv4(text);
  return labels.every(
    (label) => /^[a-z0-9_]([a-z0-9_-]{0,61}[a-z0-9_])?$/.test(label) && (label.slice(2, 4) !== "--" || label.startsWith("xn--")),
  );
}

/** Splits `host[:port]` or `[v6][:port]`; null when the brackets or the port can't be read. */
function splitHostPort(text: string): { host: string; v6: boolean; port: string | null } | null {
  if (text.startsWith("[")) {
    const close = text.indexOf("]");
    if (close < 0) return null;
    const after = text.slice(close + 1);
    if (after !== "" && !after.startsWith(":")) return null;
    return { host: text.slice(1, close), v6: true, port: after === "" ? null : after.slice(1) };
  }
  const colon = text.lastIndexOf(":");
  return colon < 0 ? { host: text, v6: false, port: null } : { host: text.slice(0, colon), v6: false, port: text.slice(colon + 1) };
}

/** A host (a probe's `host`) already in normal form: a name, an IPv4 address or a bracketed IPv6 one, maybe a port. */
export function isNormalHost(text: string): boolean {
  const split = splitHostPort(text);
  if (split === null || (split.port !== null && !isNormalPort(split.port))) return false;
  return split.v6 ? isNormalIpv6(split.host) : isNormalName(split.host);
}

/** A socket address (a probe's `local`) in normal form: `a.b.c.d:port` or `[v6]:port`, an IPv4-mapped one as IPv4. */
export function isNormalSocketAddress(text: string): boolean {
  const split = splitHostPort(text);
  if (split === null || split.port === null || !isNormalPort(split.port)) return false;
  return split.v6 ? isNormalIpv6(split.host) : isIpv4(split.host);
}

/** A route's URL, taken apart and put in normal form. */
export interface RouteAddress {
  scheme: "http" | "https";
  /** Whether the host is an IP address (an address route can only be one). */
  isAddress: boolean;
  /** The host as a probe names it: the port only when it isn't the scheme's default. */
  host: string;
  /** The address and port, as a probe's `local` names them; null for a name. */
  socket: string | null;
}

/** Reads an http(s) URL's scheme, host and port into the normal form; null when it isn't such a URL. */
export function routeAddress(url: string): RouteAddress | null {
  const match = /^(https?):\/\/(\[[^\]]*\]|[^/:?#[\]]+)(?::(\d+))?(?:[/?#].*)?$/i.exec(url);
  if (!match) return null;
  const scheme = match[1]!.toLowerCase() as "http" | "https";
  const defaultPort = scheme === "https" ? 443 : 80;
  const port = match[3] === undefined ? defaultPort : Number(match[3]);
  if (!Number.isInteger(port) || port < 1 || port > 65_535) return null;
  const rawHost = match[2]!;
  let host: string;
  let isAddress: boolean;
  if (rawHost.startsWith("[")) {
    const groups = parseIpv6(rawHost.slice(1, -1));
    if (groups === null || isIpv4Mapped(groups)) return null;
    host = `[${formatIpv6(groups)}]`;
    isAddress = true;
  } else {
    host = rawHost.toLowerCase();
    if (!isNormalName(host)) return null;
    isAddress = isIpv4(host);
  }
  return {
    scheme,
    isAddress,
    host: port === defaultPort ? host : `${host}:${port}`,
    socket: isAddress ? `${host}:${port}` : null,
  };
}
