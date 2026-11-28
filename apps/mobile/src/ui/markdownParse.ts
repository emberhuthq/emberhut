// The markdown Emberhut and Claude write, read the way the reference client's chat reads it: paragraphs, lists, headings, quotes, fenced
// code, tables and rules as blocks; bold, italics, code, links, bare addresses and task numbers inside them.

import { TASK_KEY_PATTERN, TASK_KEY_PREFIX } from "@/api/wire";

/** A task key at the start of what's left, not run into a following letter, digit or underscore. */
const taskRef = new RegExp(`^${TASK_KEY_PATTERN}(?![\\p{L}\\p{N}_])`, "u");

export type Block =
  | { kind: "para"; text: string }
  | { kind: "item"; marker: string; depth: number; text: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "quote"; text: string }
  | { kind: "code"; lang: string; text: string }
  | { kind: "table"; rows: string[][] }
  | { kind: "image"; url: string; alt: string }
  | { kind: "rule" };

export interface Span {
  text: string;
  bold?: boolean;
  italic?: boolean;
  code?: boolean;
  /** An address, or emberhut://task/<id> for a task number. */
  link?: string;
}

export interface BlockOptions {
  /** Two sentences a paragraph: a wall of text broken up, as the chat does with Emberhut's answers. */
  split?: boolean;
  /** A paragraph's lines stay on their own lines, as typed (your own messages); otherwise they join, as markdown does. */
  breaks?: boolean;
}

/** "| a | b |" as its cells; the "|---|:--:|" divider as none. "\|" is a bar inside a cell, "<br>" a new line. */
export function tableCells(line: string): string[] | null {
  let t = line.trim();
  if (!t.startsWith("|")) return null;
  t = t.slice(1);
  if (t.endsWith("|") && !t.endsWith("\\|")) t = t.slice(0, -1);
  const cells = t
    .replace(/\\\|/g, "\u0002")
    .split("|")
    .map((c) =>
      c
        .replace(/\u0002/g, "|")
        .replace(/<br\s*\/?>/g, "\n")
        .trim(),
    );
  return cells.every((c) => c !== "" && /^[-: ]+$/.test(c)) ? [] : cells;
}

export function parseBlocks(text: string, options: BlockOptions = {}): Block[] {
  const out: Block[] = [];
  let para: string[] = [];
  let table: string[][] = [];
  let quote: string[] = [];
  let code: { lang: string; lines: string[] } | null = null;

  const flushPara = () => {
    const joined = para.join(options.breaks ? "\n" : " ").trim();
    para = [];
    if (joined === "") return;
    if (!options.split) {
      out.push({ kind: "para", text: joined });
      return;
    }
    const sentences = joined.replace(/([.!?])\s+(?=[A-Z"“(\[])/g, "$1\u0001").split("\u0001");
    for (let i = 0; i < sentences.length; i += 2) out.push({ kind: "para", text: sentences.slice(i, i + 2).join(" ") });
  };
  const flushTable = () => {
    if (table.length > 0) out.push({ kind: "table", rows: table });
    table = [];
  };
  const flushQuote = () => {
    const q = quote.join("\n").trim();
    if (q !== "") out.push({ kind: "quote", text: q });
    quote = [];
  };

  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (code === null) {
      const quoted = /^>\s?(.*)$/.exec(line);
      if (quoted) {
        flushPara();
        flushTable();
        quote.push(quoted[1] ?? "");
        continue;
      }
    }
    flushQuote();
    if (line.startsWith("```") || line.startsWith("~~~")) {
      if (code !== null) {
        out.push({ kind: "code", lang: code.lang, text: code.lines.join("\n") });
        code = null;
      } else {
        flushPara();
        flushTable();
        code = { lang: line.slice(3).trim().toLowerCase(), lines: [] };
      }
      continue;
    }
    if (code !== null) {
      code.lines.push(raw);
      continue;
    }
    const row = tableCells(line);
    if (row !== null) {
      flushPara();
      if (row.length > 0) table.push(row);
      continue;
    }
    flushTable();
    if (line === "") {
      flushPara();
      continue;
    }
    const heading = /^(#{1,6})\s+(.+)$/.exec(line);
    if (heading) {
      flushPara();
      out.push({ kind: "heading", level: heading[1]!.length, text: heading[2]! });
      continue;
    }
    if (line === "---" || line === "***" || line === "___") {
      flushPara();
      out.push({ kind: "rule" });
      continue;
    }
    const image = /^!\[([^\]]*)\]\(([^)\s]+)\)$/.exec(line);
    if (image) {
      flushPara();
      out.push({ kind: "image", alt: image[1]!, url: image[2]! });
      continue;
    }
    const item = /^(?:[-•*]|(\d+)[.)])\s+(.+)$/.exec(line);
    if (item) {
      flushPara();
      const indent = raw.length - raw.trimStart().length;
      out.push({ kind: "item", marker: item[1] ? `${item[1]}.` : "•", depth: Math.min(Math.floor(indent / 2), 3), text: item[2]! });
      continue;
    }
    para.push(line);
  }
  flushPara();
  flushTable();
  flushQuote();
  // An unclosed fence (an answer still arriving) still shows.
  if (code !== null) out.push({ kind: "code", lang: code.lang, text: code.lines.join("\n") });
  return out;
}

export interface InlineOptions {
  /** A task key (task 123's) links to its task (emberhut://task/123). */
  taskRefs?: boolean;
}

type Style = Omit<Span, "text">;

const word = /[\p{L}\p{N}_]/u;
const trailing = /[.,;:!?'"]+$/;

/** A run of text as styled spans, in order; their texts joined are the words shown. */
export function parseInline(text: string, options: InlineOptions = {}): Span[] {
  const spans: Span[] = [];
  scan(text, {}, options.taskRefs ?? false, spans);
  return merge(spans);
}

function scan(text: string, style: Style, taskRefs: boolean, out: Span[]): void {
  let plain = "";
  const flush = () => {
    if (plain !== "") out.push({ ...style, text: plain });
    plain = "";
  };
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i);
    const before = i > 0 ? text[i - 1]! : "";
    const ch = text[i]!;

    if (ch === "`") {
      const end = text.indexOf("`", i + 1);
      if (end > i + 1) {
        flush();
        out.push({ ...style, code: true, text: text.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    if (rest.startsWith("**") || rest.startsWith("__")) {
      const mark = rest.slice(0, 2);
      const end = text.indexOf(mark, i + 2);
      if (end > i + 2) {
        flush();
        scan(text.slice(i + 2, end), { ...style, bold: true }, taskRefs, out);
        i = end + 2;
        continue;
      }
    }
    if (ch === "[") {
      const link = /^\[([^\]]+)\]\(([^)\s]+)\)/.exec(rest);
      if (link) {
        flush();
        scan(link[1]!, { ...style, link: link[2]! }, false, out);
        i += link[0].length;
        continue;
      }
    }
    if (style.link === undefined && (rest.startsWith("https://") || rest.startsWith("http://")) && !word.test(before)) {
      const raw = /^\S+/.exec(rest)![0];
      let url = raw.replace(trailing, "");
      // A closing bracket belongs to the address only when it opened one.
      while (url.endsWith(")") && (url.match(/\(/g)?.length ?? 0) < (url.match(/\)/g)?.length ?? 0))
        url = url.slice(0, -1).replace(trailing, "");
      flush();
      out.push({ ...style, link: url, text: url });
      i += url.length;
      continue;
    }
    if ((ch === "*" || ch === "_") && !word.test(before) && text[i + 1] !== ch && text[i + 1] !== undefined && text[i + 1] !== " ") {
      const end = closing(text, ch, i + 1);
      if (end > 0) {
        flush();
        scan(text.slice(i + 1, end), { ...style, italic: true }, taskRefs, out);
        i = end + 1;
        continue;
      }
    }
    if (taskRefs && style.link === undefined && ch === TASK_KEY_PREFIX[0] && !/[\p{L}\p{N}_/-]/u.test(before)) {
      const ref = taskRef.exec(rest);
      if (ref) {
        flush();
        out.push({ ...style, link: `emberhut://task/${ref[1]}`, text: ref[0] });
        i += ref[0].length;
        continue;
      }
    }
    plain += ch;
    i += 1;
  }
  flush();
}

/** Where a single * or _ emphasis closes: a mark after a non-space, not inside a word. */
function closing(text: string, mark: string, from: number): number {
  for (let j = from + 1; j < text.length; j++) {
    if (text[j] !== mark) continue;
    if (text[j - 1] === " " || text[j + 1] === mark) continue;
    if (word.test(text[j + 1] ?? "")) continue;
    return j;
  }
  return -1;
}

function merge(spans: Span[]): Span[] {
  const out: Span[] = [];
  for (const s of spans) {
    const last = out.at(-1);
    if (last && last.bold === s.bold && last.italic === s.italic && last.code === s.code && last.link === s.link) last.text += s.text;
    else out.push({ ...s });
  }
  return out;
}

/** The words alone: what a preview or a copy shows. */
export function plainText(text: string): string {
  return parseInline(text)
    .map((s) => s.text)
    .join("");
}

/** Right to left when the first letter is Arabic or Hebrew; digits, symbols and marks don't count. */
export function isRightToLeft(text: string): boolean {
  for (const ch of text) {
    const c = ch.codePointAt(0)!;
    if ((c >= 0x0590 && c <= 0x08ff) || (c >= 0xfb1d && c <= 0xfdff) || (c >= 0xfe70 && c <= 0xfeff)) return true;
    if ((c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a) || (c >= 0xc0 && c <= 0x24f)) return false;
  }
  return false;
}

/** A message as it pastes into WhatsApp or Slack: *bold*, links as "text (address)", no heading marks. */
export function copyText(markdown: string): string {
  return markdown
    .replace(/\*\*(.+?)\*\*/g, "*$1*")
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_, label: string, url: string) =>
      url.startsWith("mailto:") || url.includes(label) ? label : `${label} (${url})`,
    )
    .replace(/^#{1,6}\s+/gm, "");
}
