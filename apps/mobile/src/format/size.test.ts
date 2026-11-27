import { describe, expect, it } from "vitest";
import { fileSize, gb, gbOf, percent, share } from "./size";

const GB = 1024 ** 3;

describe("gb", () => {
  it("says bytes as GB with one decimal", () => {
    expect(gb(0)).toBe("0.0 GB");
    expect(gb(12.34 * GB)).toBe("12.3 GB");
    expect(gb(66510815232)).toBe("61.9 GB");
    expect(gb(512 * 1024 * 1024)).toBe("0.5 GB");
  });

  it("holds sizes past 32 bits", () => {
    expect(gb(768174227456)).toBe("715.4 GB");
  });

  it("says used of total", () => {
    expect(gbOf(10178637824, 66510815232)).toBe("9.5 of 61.9 GB");
  });
});

describe("percent and share", () => {
  it("rounds a share to a whole percent", () => {
    expect(percent(0)).toBe("0%");
    expect(percent(0.125)).toBe("13%");
    expect(percent(1)).toBe("100%");
  });

  it("keeps a share within 0…1, and nothing to divide by is empty", () => {
    expect(share(5, 10)).toBe(0.5);
    expect(share(12, 10)).toBe(1);
    expect(share(5, 0)).toBe(0);
  });
});

describe("fileSize", () => {
  it("says sizes in binary units as the desktop app does: one decimal under 10, whole from there", () => {
    expect(fileSize(0)).toBe("0 B");
    expect(fileSize(1023)).toBe("1023 B");
    expect(fileSize(1024)).toBe("1.0 KiB");
    expect(fileSize(1536)).toBe("1.5 KiB");
    expect(fileSize(12 * 1024 + 100)).toBe("12 KiB");
    expect(fileSize(3_565_158)).toBe("3.4 MiB");
    expect(fileSize(500 * 1024 * 1024)).toBe("500 MiB");
    expect(fileSize(1_288_490_189)).toBe("1.2 GiB");
  });
});
