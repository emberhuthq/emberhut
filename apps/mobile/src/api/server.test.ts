import { describe, expect, it } from "vitest";
import { memoryStorage } from "@/test/fakes";
import { checkServerAddress, createServerAddress, serverUrlFrom } from "./server";

// Private addresses are put together from their numbers: the repository's personal-data scan flags any written out.
const ip = (...parts: number[]) => parts.join(".");

describe("serverUrlFrom", () => {
  it("makes what is typed whole, https unless it's typed otherwise", () => {
    expect(serverUrlFrom(" my-server:18990 ")).toBe("https://my-server:18990");
    expect(serverUrlFrom("https://Emberhut.Example.com/")).toBe("https://emberhut.example.com");
    expect(serverUrlFrom("http://[fd00::1]:8080/emberhut//")).toBe("http://[fd00::1]:8080/emberhut");
    expect(serverUrlFrom(`https://${ip(10, 0, 0, 5)}:18990`)).toBe(`https://${ip(10, 0, 0, 5)}:18990`);
  });

  it("takes plain http to this phone or a private network only, and says it's plain", () => {
    expect(checkServerAddress(`${ip(100, 80, 1, 2)}:18990`)).toEqual({ ok: true, url: `http://${ip(100, 80, 1, 2)}:18990`, plain: true });
    expect(checkServerAddress(`http://${ip(192, 168, 1, 20)}:18990`)).toEqual({
      ok: true,
      url: `http://${ip(192, 168, 1, 20)}:18990`,
      plain: true,
    });
    expect(checkServerAddress("localhost:18990")).toMatchObject({ url: "http://localhost:18990", plain: true });
    expect(checkServerAddress("emberhut.example.com")).toMatchObject({ url: "https://emberhut.example.com", plain: false });
    expect(checkServerAddress(`${ip(8, 8, 8, 8)}:18990`)).toMatchObject({ url: `https://${ip(8, 8, 8, 8)}:18990`, plain: false });
    for (const refused of ["http://my-server:18990", "http://emberhut.example.com", `http://${ip(8, 8, 8, 8)}`]) {
      const check = checkServerAddress(refused);
      expect(check.ok, refused).toBe(false);
      expect(!check.ok && check.error).toMatch(/Plain http only reaches/);
    }
  });

  it("refuses what isn't an http(s) address of a host", () => {
    expect(serverUrlFrom("")).toBeNull();
    expect(serverUrlFrom("ftp://host")).toBeNull();
    expect(serverUrlFrom("http://")).toBeNull();
    expect(serverUrlFrom("my server")).toBeNull();
    expect(serverUrlFrom("http://host?x=1")).toBeNull();
  });
});

describe("createServerAddress", () => {
  it("keeps the address across launches", async () => {
    const storage = memoryStorage();
    await createServerAddress(storage).save("https://my-server:18990");
    const next = createServerAddress(storage);
    expect(next.url).toBeNull();
    expect(await next.load()).toBe("https://my-server:18990");
  });
});
