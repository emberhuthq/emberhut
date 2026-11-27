import { describe, expect, it } from "vitest";
import { ASSISTANT, taskKey, type Decision, type Issue, type EmberhutState, type Project, type WorkRow } from "@/api";
import {
  answerLabel,
  askText,
  byUrgency,
  cardAction,
  cardAnswers,
  draftToCopy,
  needsCount,
  needsGroups,
  needsRows,
  projectGroups,
  splitAsk,
  whenOf,
} from "./model";

const now = new Date(2026, 10, 24, 10);

function issue(id: number, fields: Partial<Issue> = {}): Issue {
  return {
    id,
    key: taskKey(id),
    title: `Task ${id}`,
    owner: ASSISTANT,
    status: "waiting",
    priority: 0,
    today: false,
    updated: "",
    ...fields,
  };
}

function row(task: number, fields: Partial<WorkRow> = {}): WorkRow {
  return { id: `t${task}`, kind: "task", task, title: `Task ${task}`, who: "Emberhut", state: "waiting", action: "Say yes", ...fields };
}

const state = (working: WorkRow[], issues: Issue[]): EmberhutState => ({ working, issues }) as EmberhutState;

describe("when you have to decide", () => {
  it("puts overdue and today in Today, then the week, the month and later; no date is this week", () => {
    expect(whenOf("2026-11-20", now)).toBe("today");
    expect(whenOf("2026-11-24", now)).toBe("today");
    expect(whenOf("2026-11-30", now)).toBe("week");
    expect(whenOf("2026-12-01", now)).toBe("month");
    expect(whenOf("2027-01-10", now)).toBe("later");
    expect(whenOf(null, now)).toBe("week");
  });
});

describe("Needs you cards", () => {
  it("leaves out what you just acted on, what no longer waits, and a row that copies another's ask", () => {
    const issues = [
      issue(1),
      issue(2),
      issue(3, { decision: null, action: `One yes on ${taskKey(1)}` }),
      issue(4, { action: `See ${taskKey(9)}` }),
      issue(9, { status: "done" }),
    ];
    const rows = [row(1), row(2), row(3), row(4), row(5, { state: "running" })];
    expect(needsRows(state(rows, issues), new Set([2])).map((r) => r.task)).toEqual([1]);
  });

  it("counts what waits on you for the badge, Later and paused projects left out", () => {
    const rows = [row(1, { decideBy: "2026-11-24" }), row(2, { decideBy: "2027-03-01" }), row(3, { paused: true }), row(4)];
    expect(needsCount(state(rows, []), new Set(), now)).toBe(2);
  });

  it("counts the approvals waiting on you in the badge, not those waiting on others", () => {
    const pending = (id: number, waitsOnOwner: boolean) =>
      ({
        id,
        agent: "client-a",
        kind: "post",
        target: "t",
        summary: "s",
        payloadHash: "",
        approver: "owner",
        waitsOnOwner,
        state: "pending",
        createdAt: "",
      }) as const;
    const withApprovals = { ...state([row(1)], []), approvals: [pending(1, true), pending(2, false), pending(3, true)] } as EmberhutState;
    expect(needsCount(withApprovals, new Set(), now)).toBe(3);
  });

  it("badges exactly what the page's sections above Later hold", () => {
    const issues = [issue(1), issue(2, { action: `One yes on ${taskKey(1)}` }), issue(3, { status: "done" }), issue(5)];
    const rows = [row(1, { decideBy: "2026-11-24" }), row(2), row(3), row(4, { decideBy: "2026-12-10" }), row(5, { paused: true }), row(6)];
    const shown = needsGroups(needsRows(state(rows, issues), new Set([6])), now).filter((g) => g.when !== "later");
    expect(needsCount(state(rows, issues), new Set([6]), now)).toBe(shown.reduce((n, g) => n + g.rows.length, 0));
  });

  it("groups by when, a quick yes first, then the soonest, then the newest ask", () => {
    const rows = [
      row(1, { need: "do", decideBy: "2026-11-26" }),
      row(2, { need: "choose", decideBy: "2026-11-25" }),
      row(3, { need: "approve", decideBy: "2026-11-29" }),
      row(4, { need: "choose", decideBy: "2026-11-25", asked: "2026-11-23T10:00:00Z" }),
      row(5, { decideBy: "2026-11-24" }),
      row(6, { decideBy: "2027-02-01" }),
    ];
    expect(needsGroups(rows, now).map((g) => [g.when, g.rows.map((r) => r.task)])).toEqual([
      ["today", [5]],
      ["week", [3, 4, 2, 1]],
      ["later", [6]],
    ]);
  });

  it("groups by project, the one with the most urgent ask first, Other last, Later left out", () => {
    const projects = [
      { id: 1, name: "Garden" },
      { id: 2, name: "Studio" },
      { id: 4, name: "Bakery" },
    ] as Project[];
    const rows = [
      row(1, { pid: 2, decideBy: "2026-11-30" }),
      row(2, { pid: 1, decideBy: "2026-11-28" }),
      row(3, { pid: 2, decideBy: "2026-11-20", need: "do" }),
      row(4, { pid: null, decideBy: "2026-11-21" }),
      row(5, { pid: 1, decideBy: "2026-12-10" }),
      row(6, { pid: 4, decideBy: "2027-02-01" }),
      row(7, { pid: 99, decideBy: "2026-11-25" }),
      row(8, { pid: 1, need: "approve" }),
      row(9, { pid: 1, paused: true }),
    ];
    expect(projectGroups(rows, projects, now).map((g) => [g.name, g.rows.map((r) => r.task)])).toEqual([
      ["Studio", [3, 1]],
      ["Garden", [2, 8, 5]],
      ["Other", [4, 7]],
    ]);
  });

  it("badges exactly what the project sections hold", () => {
    const rows = [
      row(1, { pid: 1, decideBy: "2026-11-24" }),
      row(2, { decideBy: "2027-03-01" }),
      row(3, { paused: true }),
      row(4, { pid: 2 }),
    ];
    const shown = projectGroups(needsRows(state(rows, []), new Set()), [{ id: 1, name: "Garden" }] as Project[], now);
    expect(needsCount(state(rows, []), new Set(), now)).toBe(shown.reduce((n, g) => n + g.rows.length, 0));
  });

  it("orders by urgency: overdue first, then the soonest, a quick yes before a pick on the same day", () => {
    const rows = [
      row(1, { decideBy: "2026-11-26", need: "choose" }),
      row(2, { decideBy: "2026-11-26", need: "approve" }),
      row(3, { decideBy: "2026-11-19" }),
      row(4),
    ];
    expect(byUrgency(rows, now).map((r) => r.task)).toEqual([3, 2, 1, 4]);
  });

  it("answers a quick yes on the card, and not a pick to read, a stale ask or one without an action", () => {
    const fresh = { checked: "2026-11-24T08:00:00Z" };
    const issues = [issue(1, { decision: { question: "Ship it?", options: ["Approve", "No"] } })];
    expect(cardAnswers(row(1, { need: "approve", ...fresh }), issues, now)).toMatchObject({ task: 1, options: ["Approve", "No"] });
    expect(cardAnswers(row(1, { need: "choose", ...fresh }), issues, now)).toBeNull();
    expect(cardAnswers(row(1, { need: "approve", checked: "2026-11-23T08:00:00Z" }), issues, now)).toBeNull();
    expect(cardAnswers(row(1, { need: "approve", action: null, ...fresh }), issues, now)).toBeNull();
  });

  it("answers the task an action names, and offers I did it when there is only something to do", () => {
    const fresh = { checked: "2026-11-24T08:00:00Z", need: "do" as const };
    const issues = [
      issue(7, { action: `One yes on ${taskKey(8)}` }),
      issue(8, { decision: { question: "Pay?", options: ["Pay"] } }),
      issue(9),
    ];
    expect(cardAnswers(row(7, fresh), issues, now)).toMatchObject({ task: 8, options: ["Pay"] });
    expect(cardAnswers(row(9, fresh), issues, now)).toEqual({ task: 9, options: [], decision: null });
    expect(cardAction(row(7), issues)).toBe("Pay?");
    expect(cardAction(row(9, { action: "Log in to the bank" }), issues)).toBe("Log in to the bank");
  });

  it("leaves an X reply to its page, to post first", () => {
    const fresh = { checked: "2026-11-24T08:00:00Z", need: "do" as const };
    const reply = {
      kind: "x_reply",
      x: { post_url: "https://x.com/a/status/1", post_id: "1", replies: [] },
    } as unknown as WorkRow["reply"];
    expect(cardAnswers(row(9, { ...fresh, reply }), [issue(9)], now)).toBeNull();
  });

  it("says Done as I did it", () => {
    expect(answerLabel("Done")).toBe("I did it");
    expect(answerLabel("Approve")).toBe("Approve");
  });
});

describe("the ask's text", () => {
  const message = "Hi Abdullah, the invoice for October is attached. Could you confirm the amount before Friday?";

  it("cuts the draft after its label line, keeping what comes after", () => {
    const text = splitAsk(`Abdullah asked about the invoice.\nDraft reply to Abdullah:\n"${message}"\nSend it if it reads right.`);
    expect(text).toEqual({
      summary: "Abdullah asked about the invoice.",
      draftLabel: "Draft reply to Abdullah",
      draft: message,
      after: "Send it if it reads right.",
    });
  });

  it("takes a quote that opens a line as the draft, and leaves a short quoted phrase in the summary", () => {
    expect(splitAsk(`They wrote back.\n“${message}”`)).toMatchObject({ summary: "They wrote back.", draft: message });
    expect(splitAsk('Should I call it "done"?')).toMatchObject({ summary: 'Should I call it "done"?', draft: null });
  });

  it("uses the draft field when the server sends one", () => {
    const decision: Decision = { question: "Send this?", options: ["Send it"], draft: message };
    expect(askText(decision)).toEqual({ summary: "Send this?", draftLabel: null, draft: message, after: "" });
  });

  it("copies the draft only for an answer that sends it", () => {
    const decision: Decision = { question: "Send this?", options: ["Send it", "Leave it"], draft: message };
    expect(draftToCopy("Send it", decision)).toBe(message);
    expect(draftToCopy("Leave it", decision)).toBeNull();
    expect(draftToCopy("Send it", null)).toBeNull();
  });
});
