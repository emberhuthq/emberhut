import { describe, expect, it } from "vitest";
import { qrPath } from "./qrPath";

/** The dark modules the path draws, as "row,col". */
function modules(path: string): Set<string> {
  const dark = new Set<string>();
  for (const [, x, y, w] of path.matchAll(/M(\d+) (\d+)h(\d+)v1h-\d+z/g))
    for (let i = 0; i < Number(w); i += 1) dark.add(`${y},${Number(x) + i}`);
  return dark;
}

describe("the QR code's path", () => {
  const link = "https://emberhut.com/pair#id=3j6rajbbrd2yrrvdkoq2mouz5aqxwh5c&code=ABCD-EFGH&route=https%3A%2F%2Femberhut.example";

  it("draws a version that fits the link, with its three finder patterns", () => {
    const qr = qrPath(link);
    expect(qr).not.toBeNull();
    const { size, path } = qr!;
    expect((size - 17) % 4).toBe(0);
    const dark = modules(path);
    // Each finder pattern's outer ring and centre: top left, top right, bottom left.
    const corners: [number, number][] = [
      [0, 0],
      [0, size - 7],
      [size - 7, 0],
    ];
    for (const [r, c] of corners) {
      for (let i = 0; i < 7; i += 1) {
        expect(dark.has(`${r},${c + i}`)).toBe(true);
        expect(dark.has(`${r + 6},${c + i}`)).toBe(true);
      }
      expect(dark.has(`${r + 3},${c + 3}`)).toBe(true);
      expect(dark.has(`${r + 1},${c + 1}`)).toBe(false);
    }
  });

  it("holds the longest link the contract allows", () => {
    expect(qrPath(`https://emberhut.com/pair#${"a".repeat(2_000 - 22)}`)?.size).toBeLessThanOrEqual(177);
  });

  it("draws nothing for empty text or text past ASCII", () => {
    expect(qrPath("")).toBeNull();
    expect(qrPath("https://emberhut.com/pair#code=é")).toBeNull();
  });
});
