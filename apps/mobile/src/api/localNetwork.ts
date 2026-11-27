// Which servers plain http may reach: this phone itself and the private networks (a home or office LAN, a VPN's
// CGNAT addresses). Everything else is https. No imports, so app.config.ts reads the same list for iOS's ATS.

/** An IPv4 range as its first address's four numbers and its prefix length. */
export interface Ipv4Range {
  readonly start: readonly [number, number, number, number];
  readonly bits: number;
  readonly why: string;
}

/** The IPv4 ranges plain http is allowed to: loopback, RFC 1918's three private ranges, and RFC 6598's CGNAT range. */
export const privateIpv4Ranges: readonly Ipv4Range[] = [
  { start: [127, 0, 0, 0], bits: 8, why: "loopback" },
  { start: [10, 0, 0, 0], bits: 8, why: "private (RFC 1918)" },
  { start: [172, 16, 0, 0], bits: 12, why: "private (RFC 1918)" },
  { start: [192, 168, 0, 0], bits: 16, why: "private (RFC 1918)" },
  { start: [100, 64, 0, 0], bits: 10, why: "shared address space (RFC 6598), as VPNs like Tailscale use" },
];

/** IPv6's loopback, and its unique local range (RFC 4193) that VPNs and home networks hand out. */
export const privateIpv6Ranges = ["::1/128", "fc00::/7"] as const;

/** A range as CIDR text (its first address, a slash, the prefix length), as iOS's ATS exceptions take it. */
export const cidrOf = (range: Ipv4Range): string => `${range.start.join(".")}/${range.bits}`;

function ipv4(host: string): number | null {
  const parts = host.split(".");
  if (parts.length !== 4 || !parts.every((p) => /^\d{1,3}$/.test(p) && Number(p) <= 255)) return null;
  return parts.reduce((n, p) => n * 256 + Number(p), 0);
}

const inRange = (address: number, range: Ipv4Range): boolean => {
  const start = range.start.reduce((n, p) => n * 256 + p, 0);
  const size = 2 ** (32 - range.bits);
  return address >= start && address < start + size;
};

function privateIpv6(host: string): boolean {
  const address = host.replace(/^\[|\]$/g, "").toLowerCase();
  if (!/^[0-9a-f:]+$/.test(address) || !address.includes(":")) return false;
  const groups = address.split(":").filter((g) => g !== "");
  const whole = address.includes("::") || groups.length === 8;
  const last = groups.at(-1);
  // ::1, however it's spelled: every group 0 but the last, which is 1.
  if (whole && last !== undefined && Number.parseInt(last, 16) === 1 && groups.slice(0, -1).every((g) => Number.parseInt(g, 16) === 0))
    return true;
  const first = address.startsWith("::") ? 0 : Number.parseInt(groups[0] ?? "", 16);
  return (first & 0xfe00) === 0xfc00;
}

/**
 * Whether plain http to this host stays on this phone or a private network: localhost, or an IP literal in a private,
 * loopback or CGNAT range. A host name never is: the name may resolve anywhere.
 */
export function isLocalHost(host: string): boolean {
  const h = host.toLowerCase();
  if (h === "localhost") return true;
  const v4 = ipv4(h);
  if (v4 !== null) return privateIpv4Ranges.some((range) => inRange(v4, range));
  return h.startsWith("[") && privateIpv6(h);
}
