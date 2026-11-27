import { inflateRawSync } from "node:zlib";
import { describe, expect, it } from "vitest";
import { zip } from "./zip-file";

const files = [
  { name: "manifest.json", data: Buffer.from('{"version":"0.1.0"}') },
  { name: "background.js", data: Buffer.from("x".repeat(2_000)) },
  { name: "icons/emberhut-16.png", data: Buffer.from([1, 2, 3]) },
];

/** The entries of a zip, read from its central directory: name, method and contents. */
function read(bytes: Buffer): { name: string; method: number; data: Buffer }[] {
  const end = bytes.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  const count = bytes.readUInt16LE(end + 10);
  let at = bytes.readUInt32LE(end + 16);
  const out = [];
  for (let i = 0; i < count; i++) {
    const method = bytes.readUInt16LE(at + 10);
    const size = bytes.readUInt32LE(at + 20);
    const nameLength = bytes.readUInt16LE(at + 28);
    const local = bytes.readUInt32LE(at + 42);
    const name = bytes.subarray(at + 46, at + 46 + nameLength).toString("utf8");
    const start = local + 30 + bytes.readUInt16LE(local + 26) + bytes.readUInt16LE(local + 28);
    const body = bytes.subarray(start, start + size);
    out.push({ name, method, data: method === 8 ? inflateRawSync(body) : Buffer.from(body) });
    at += 46 + nameLength;
  }
  return out;
}

describe("the zip writer", () => {
  it("writes the same bytes from the same files, in any order", () => {
    expect(zip(files).equals(zip([...files].reverse()))).toBe(true);
  });

  it("holds every file, sorted, deflated when that's smaller", () => {
    const entries = read(zip(files));
    expect(entries.map((e) => e.name)).toEqual(["background.js", "icons/emberhut-16.png", "manifest.json"]);
    expect(entries[0]?.method).toBe(8);
    expect(entries[1]?.method).toBe(0);
    expect(entries[2]?.data.toString()).toBe('{"version":"0.1.0"}');
  });

  it("refuses a path outside the folder", () => {
    expect(() => zip([{ name: "../evil", data: Buffer.from("") }])).toThrow(/outside/);
    expect(() => zip([{ name: "/etc/x", data: Buffer.from("") }])).toThrow(/outside/);
  });
});
