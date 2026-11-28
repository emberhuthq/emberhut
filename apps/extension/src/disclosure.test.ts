import { describe, expect, it } from "vitest";
import { SHORT, disclosure } from "./disclosure";

describe("the disclosure", () => {
  it("names the hut, what goes to it and on to the AI model, and what never goes", () => {
    const text = disclosure("https://hut.example.com").join(" ");
    expect(text).toContain("https://hut.example.com");
    for (const word of [
      "addresses",
      "screenshots",
      "tab group",
      "runs scripts",
      "AI model",
      "cookies",
      "password",
      "Nothing goes to Symploc",
    ]) {
      expect(text).toContain(word);
    }
    expect(SHORT).toMatch(/AI model/);
  });
});
