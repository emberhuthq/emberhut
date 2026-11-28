import { describe, expect, it } from "vitest";
import { taskKey } from "@/api";
import { copyText, isRightToLeft, parseBlocks, parseInline, plainText, tableCells } from "./markdownParse";

describe("parseBlocks", () => {
  it("joins a paragraph's lines and splits paragraphs at blank lines", () => {
    expect(parseBlocks("one\ntwo\n\nthree")).toEqual([
      { kind: "para", text: "one two" },
      { kind: "para", text: "three" },
    ]);
  });

  it("keeps each line break of a paragraph when asked, as typed", () => {
    expect(parseBlocks("Hi\nthree things:\nmilk\n\nthanks", { breaks: true })).toEqual([
      { kind: "para", text: "Hi\nthree things:\nmilk" },
      { kind: "para", text: "thanks" },
    ]);
  });

  it("breaks a wall of text every two sentences when asked", () => {
    const text = "First one. Second one! Third one? Fourth.";
    expect(parseBlocks(text, { split: true }).map((b) => (b.kind === "para" ? b.text : ""))).toEqual([
      "First one. Second one!",
      "Third one? Fourth.",
    ]);
    expect(parseBlocks(text)).toHaveLength(1);
  });

  it("reads lists with their markers and depth", () => {
    expect(parseBlocks("- a\n  - b\n2. c")).toEqual([
      { kind: "item", marker: "•", depth: 0, text: "a" },
      { kind: "item", marker: "•", depth: 1, text: "b" },
      { kind: "item", marker: "2.", depth: 0, text: "c" },
    ]);
  });

  it("keeps one quote per run of > lines, a bare > as a blank line inside it", () => {
    expect(parseBlocks("Two drafts:\n\n> Hey Sam,\n>\n> ready.\n\nOr shorter")).toEqual([
      { kind: "para", text: "Two drafts:" },
      { kind: "quote", text: "Hey Sam,\n\nready." },
      { kind: "para", text: "Or shorter" },
    ]);
  });

  it("takes fenced code as it is, an unclosed fence too", () => {
    expect(parseBlocks("```ts\nconst a = 1;\n  b()\n```\nafter")).toEqual([
      { kind: "code", lang: "ts", text: "const a = 1;\n  b()" },
      { kind: "para", text: "after" },
    ]);
    expect(parseBlocks("```\nstill coming")).toEqual([{ kind: "code", lang: "", text: "still coming" }]);
  });

  it("reads tables without their divider row", () => {
    expect(parseBlocks("| | **A** | B |\n|---|:--:|---|\n| x | 1 | 2 \\| 3 |")).toEqual([
      {
        kind: "table",
        rows: [
          ["", "**A**", "B"],
          ["x", "1", "2 | 3"],
        ],
      },
    ]);
  });

  it("reads headings, rules and pictures", () => {
    expect(parseBlocks("## Plan\n---\n![chart](/uploads/a.png)")).toEqual([
      { kind: "heading", level: 2, text: "Plan" },
      { kind: "rule" },
      { kind: "image", alt: "chart", url: "/uploads/a.png" },
    ]);
  });
});

describe("tableCells", () => {
  it("is null for a line that isn't a row and empty for the divider", () => {
    expect(tableCells("plain")).toBeNull();
    expect(tableCells("|---|---|")).toEqual([]);
    expect(tableCells("| a<br>b | c |")).toEqual(["a\nb", "c"]);
  });
});

describe("parseInline", () => {
  it("styles bold, italics and code", () => {
    expect(parseInline("a **b** _c_ *d* `e*f`")).toEqual([
      { text: "a " },
      { text: "b", bold: true },
      { text: " " },
      { text: "c", italic: true },
      { text: " " },
      { text: "d", italic: true },
      { text: " " },
      { text: "e*f", code: true },
    ]);
  });

  it("leaves marks inside words and lone stars alone", () => {
    expect(plainText("snake_case_name and 2 * 3 * 4")).toBe("snake_case_name and 2 * 3 * 4");
    expect(parseInline("snake_case_name")).toEqual([{ text: "snake_case_name" }]);
  });

  it("reads links and bare addresses, without the sentence's punctuation", () => {
    expect(parseInline("See [the runbook](https://example.com/run) and https://github.com/x/pull/391.")).toEqual([
      { text: "See " },
      { text: "the runbook", link: "https://example.com/run" },
      { text: " and " },
      { text: "https://github.com/x/pull/391", link: "https://github.com/x/pull/391" },
      { text: "." },
    ]);
    expect(parseInline("(https://en.wikipedia.org/wiki/Emberhut_(name))")).toEqual([
      { text: "(" },
      { text: "https://en.wikipedia.org/wiki/Emberhut_(name)", link: "https://en.wikipedia.org/wiki/Emberhut_(name)" },
      { text: ")" },
    ]);
  });

  it("links task numbers only when asked, and never inside code, links or paths", () => {
    expect(parseInline(`Done on ${taskKey(3744)}.`, { taskRefs: true })).toEqual([
      { text: "Done on " },
      { text: taskKey(3744), link: "emberhut://task/3744" },
      { text: "." },
    ]);
    expect(parseInline(taskKey(12), {})).toEqual([{ text: taskKey(12) }]);
    const kept = parseInline(`\`${taskKey(1)}\` feat/${taskKey(2)} [${taskKey(3)}](https://x.y) ${taskKey(4)}x`, { taskRefs: true });
    expect(kept.filter((s) => s.link?.startsWith("emberhut://"))).toEqual([]);
  });

  it("styles nest: a link in bold", () => {
    expect(parseInline("**see [it](https://a.b)**")).toEqual([
      { text: "see ", bold: true },
      { text: "it", bold: true, link: "https://a.b" },
    ]);
  });
});

describe("helpers", () => {
  it("knows right-to-left text by its first letter", () => {
    expect(isRightToLeft("12 مرحبا")).toBe(true);
    expect(isRightToLeft("Hello مرحبا")).toBe(false);
  });

  it("copies a message for WhatsApp", () => {
    expect(copyText("## Plan\n**Bold** and [the doc](https://d.x) and [https://a.b](https://a.b)")).toBe(
      "Plan\n*Bold* and the doc (https://d.x) and https://a.b",
    );
  });
});
