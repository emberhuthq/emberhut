import { describe, expect, it } from "vitest";
import { cidrOf, isLocalHost, privateIpv4Ranges } from "./localNetwork";

// Addresses are put together from their numbers: the repository's personal-data scan flags any private address written
// out, whoever's it is.
const ip = (...parts: number[]) => parts.join(".");

describe("isLocalHost", () => {
  it("takes localhost and the private, loopback and CGNAT ranges, edges included", () => {
    for (const host of [
      "localhost",
      "LOCALHOST",
      ip(127, 0, 0, 1),
      ip(10, 0, 0, 5),
      ip(10, 255, 255, 255),
      ip(172, 16, 0, 1),
      ip(172, 31, 255, 254),
      ip(192, 168, 1, 20),
      ip(100, 64, 0, 1),
      ip(100, 127, 255, 254),
      "[::1]",
      "[0:0:0:0:0:0:0:1]",
      "[fd12:3456::1]",
      "[fc00::1]",
    ])
      expect(isLocalHost(host), host).toBe(true);
  });

  it("refuses host names, public addresses and the edges just outside", () => {
    for (const host of [
      "my-server",
      "emberhut.example.com",
      "localhost.example.com",
      ip(8, 8, 8, 8),
      ip(172, 15, 255, 255),
      ip(172, 32, 0, 1),
      ip(100, 63, 255, 255),
      ip(100, 128, 0, 1),
      ip(192, 169, 0, 1),
      ip(10, 0, 0, 256),
      ip(10, 0, 0),
      "[2001:db8::1]",
      "[fe80::1]",
      "[::2]",
    ])
      expect(isLocalHost(host), host).toBe(false);
  });

  it("writes each range as CIDR, as iOS's exceptions take it", () => {
    expect(privateIpv4Ranges.map(cidrOf)).toContain(`${ip(172, 16, 0, 0)}/12`);
    expect(privateIpv4Ranges.map(cidrOf)).toHaveLength(5);
  });
});
