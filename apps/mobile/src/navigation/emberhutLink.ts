import { linkSchemes } from "@/config/constants";

/**
 * An app link (emberhut://) as the screen it opens: a task, a project, a session, the browser,
 * the parties or one party, the pending approvals or one request.
 */
export type EmberhutLink =
  | { screen: "Task" | "Project" | "Party" | "Approval"; id: number }
  | { screen: "Session"; id: string }
  | { screen: "Browser" }
  | { screen: "Parties" }
  | { screen: "Approvals" };

const numbered = { task: "Task", project: "Project", party: "Party", approval: "Approval" } as const;

export function emberhutLink(url: string): EmberhutLink | null {
  const scheme = linkSchemes.find((s) => url.startsWith(s));
  if (scheme === undefined) return null;
  // The kind ends at the path's end, a slash, a query or a fragment: `approvalsfoo` is no kind.
  const match = /^(task|project|session|browser|party|approvals|approval)(?=$|[/?#])(?:\/([^?#]+))?/.exec(url.slice(scheme.length));
  if (!match) return null;
  const [, kind, rest] = match;
  if (kind === "browser") return { screen: "Browser" };
  if (kind === "party" && !rest) return { screen: "Parties" };
  if (kind === "approvals") return rest ? null : { screen: "Approvals" };
  if (!rest) return null;
  if (kind === "session") return { screen: "Session", id: decodeURIComponent(rest) };
  const id = Number(rest);
  return Number.isInteger(id) && id > 0 ? { screen: numbered[kind as keyof typeof numbered], id } : null;
}
