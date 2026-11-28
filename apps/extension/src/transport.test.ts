import { describe, expect, it } from "vitest";
import { plainAllowed, transportProblem } from "./transport";

describe("the hut's link", () => {
  it("takes https anywhere", () => {
    expect(transportProblem("https://hut.example.com")).toBeNull();
    expect(transportProblem("https://192.168.1.10:8443/emberhut")).toBeNull();
  });

  it("takes plain http only on this computer or a Tailscale address", () => {
    for (const ok of [
      "http://localhost:18990",
      "http://127.0.0.1:18990",
      "http://[::1]:18990",
      "http://100.64.0.5:18990",
      "http://[fd7a:115c:a1e0::1]:18990",
    ]) {
      expect(transportProblem(ok)).toBeNull();
    }
    for (const no of [
      "http://192.168.1.11:18990",
      "http://10.0.0.5",
      "http://hut.example.com",
      "http://100.63.0.1",
      "http://100.128.0.1",
      "http://hut.local",
    ]) {
      expect(transportProblem(no)).toMatch(/encrypted/);
    }
  });

  it("reads hosts as URLs give them", () => {
    expect(plainAllowed("LOCALHOST")).toBe(true);
    expect(plainAllowed("app.localhost")).toBe(true);
    expect(plainAllowed("127.1.2.3")).toBe(true);
    expect(plainAllowed("localhost.example.com")).toBe(false);
    expect(transportProblem("not a url")).toMatch(/address/);
  });
});
