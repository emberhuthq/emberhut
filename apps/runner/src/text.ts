// Short texts the transcript and the server's thread rows show: clipped values, a tool call in a few words, an edit as a
// small diff.

/** `s` as text, cut to `n` characters with an ellipsis when longer. */
export const clip = (s: unknown, n: number): string => {
  const t = typeof s === "string" ? s : s == null ? "" : JSON.stringify(s);
  return t.length > n ? `${t.slice(0, n)}…` : t;
};

type Input = Record<string, unknown> | null | undefined;
const field = (i: Input, k: string): string | undefined => {
  const v = i?.[k];
  return typeof v === "string" && v ? v : undefined;
};
const base = (p?: string) => (p ?? "").split(/[/\\]/).pop() ?? "";

/** What a tool call is doing, in a few words ("Running: npm test"). */
export function doing(name: string, i: Input): string {
  switch (name) {
    case "Bash":
      return `Running: ${clip(field(i, "description") ?? field(i, "command") ?? "", 70)}`;
    case "Read":
      return `Reading ${base(field(i, "file_path"))}`;
    case "Edit":
    case "MultiEdit":
      return `Editing ${base(field(i, "file_path"))}`;
    case "Write":
      return `Writing ${base(field(i, "file_path"))}`;
    case "Grep":
    case "Glob":
      return `Searching ${clip(field(i, "pattern") ?? "", 50)}`;
    case "WebFetch":
      return `Reading ${clip(field(i, "url") ?? "", 60)}`;
    case "WebSearch":
      return `Searching the web: ${clip(field(i, "query") ?? "", 50)}`;
    case "Agent":
    case "Task":
      return `Helper agent: ${clip(field(i, "description") ?? "", 60)}`;
    case "TodoWrite":
    case "TaskCreate":
    case "TaskUpdate":
      return "Updating its to-do list";
    default:
      return `Using ${name}`;
  }
}

/** The one line that says what a tool call acts on: its command, file, pattern, address or query. */
export const toolSummary = (name: string, i: Input): string =>
  clip(
    name === "Bash"
      ? (field(i, "command") ?? "")
      : (field(i, "file_path") ?? field(i, "pattern") ?? field(i, "url") ?? field(i, "query") ?? field(i, "description") ?? ""),
    300,
  );

/** An edit as the lines that changed, "- " before and "+ " after (a new file: its first 40 lines); null for other tools. */
export function diffOf(name: string, input: Input): string | null {
  if (!input) return null;
  const lines = (x: unknown) => (typeof x === "string" ? x : "").split("\n");
  const one = (a: unknown, b: unknown): string[] => {
    const x = lines(a),
      y = lines(b);
    let i = 0;
    while (i < x.length && i < y.length && x[i] === y[i]) i++;
    let j = 0;
    while (j < x.length - i && j < y.length - i && x[x.length - 1 - j] === y[y.length - 1 - j]) j++;
    return [...x.slice(i, x.length - j).map((t) => `- ${t}`), ...y.slice(i, y.length - j).map((t) => `+ ${t}`)];
  };
  let out: string[] | null = null;
  if (name === "Edit") out = one(input.old_string, input.new_string);
  else if (name === "MultiEdit" && Array.isArray(input.edits))
    out = (input.edits as Record<string, unknown>[]).flatMap((e, k) => [...(k ? ["…"] : []), ...one(e.old_string, e.new_string)]);
  else if (name === "Write")
    out = lines(input.content)
      .slice(0, 40)
      .map((t) => `+ ${t}`);
  return out?.length ? clip(out.join("\n"), 4000) : null;
}
