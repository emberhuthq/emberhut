import { describe, expect, it } from "vitest";
import { ApiError, type AgentRow, type Approval, type EmberhutState } from "@/api";
import {
  agentCaption,
  agentLine,
  agentName,
  agentsByRoom,
  amountText,
  approvalGroups,
  approvalsSummary,
  approvalsWaitingOnYou,
  approveQuestion,
  approverRole,
  decisionFailure,
  fileLine,
  isFixedCore,
  kindName,
  shortHash,
  agentWaitingOnYou,
  stateName,
  visible,
  waitingWords,
} from "./model";

const approval = (id: number, fields: Partial<Approval> = {}): Approval => ({
  id,
  agent: "client-a",
  kind: "post",
  target: "example blog",
  summary: `Request ${id}`,
  payloadHash: "cd".repeat(32),
  approver: "owner",
  waitsOnOwner: true,
  state: "pending",
  createdAt: "2026-11-27T10:00:00.000Z",
  ...fields,
});

const agent = (id: string, fields: Partial<AgentRow> = {}): AgentRow => ({
  id,
  name: `Agent ${id}`,
  principal: "Example Ltd",
  room: 9,
  enabled: true,
  pendingApprovals: 0,
  ...fields,
});

describe("the fixed core", () => {
  it("is payments, passwords and codes, secrets, account security and anything in your name", () => {
    for (const kind of ["payment", "credential", "secret", "account_security", "as_owner"]) expect(isFixedCore(kind)).toBe(true);
    for (const kind of ["post", "send", "merge", "deploy", "create", "other", "refund"]) expect(isFixedCore(kind)).toBe(false);
  });

  it("names each kind, and a kind this app doesn't know as the server spells it", () => {
    expect(kindName("as_owner")).toBe("In your name");
    expect(kindName("account_security")).toBe("Account security");
    expect(kindName("bulk_refund")).toBe("bulk refund");
    expect(stateName("withdrawn")).toBe("Withdrawn");
    expect(stateName("on_hold")).toBe("on_hold");
  });
});

describe("the payload as shown", () => {
  it("shortens a SHA-256 to its first 12 hex digits", () => {
    expect(shortHash("3F2A9C0B1D4E5F60718293A4B5C6D7E8F90123456789ABCDEF0123456789ABCD")).toBe("3f2a9c0b1d4e");
  });

  it("gives each file its name, size and short hash", () => {
    expect(fileLine({ name: "cover.png", size: 12_600, sha256: "ab".repeat(32) })).toBe("cover.png · 12 KiB · abababababab");
    expect(fileLine({ name: "old.pdf", size: null, sha256: "0f".repeat(32) })).toBe("old.pdf · size unknown · 0f0f0f0f0f0f");
  });

  it("says an amount exactly, the thousands grouped and the currency after", () => {
    expect(amountText(123_450, "EUR")).toBe("1,234.50 EUR");
    expect(amountText(5)).toBe("0.05");
    expect(amountText(100_000_000, "USD")).toBe("1,000,000.00 USD");
    expect(amountText(-250, "EUR")).toBe("-2.50 EUR");
  });

  it("names the agent from the request, the state's agents, or its id", () => {
    expect(agentName(approval(1, { agentName: "Example helper" }))).toBe("Example helper");
    expect(agentName(approval(1), [agent("client-a")])).toBe("Agent client-a");
    expect(agentName(approval(1, { agent: "gone" }), [agent("client-a")])).toBe("gone");
  });

  it("says who decides as a role", () => {
    expect(approverRole(approval(1))).toBe("You");
    expect(approverRole(approval(1, { approver: "example-lead", approverName: "Example Lead" }))).toBe("Example Lead, the approver");
    expect(approverRole(approval(1, { approver: "example-lead" }))).toBe("example-lead, the approver");
  });

  it("asks before an approve, naming the agent, the amount and where it goes", () => {
    const pay = approval(1, { kind: "payment", amountCents: 4_200, target: "example.com plan", agentName: "Example helper" });
    expect(approveQuestion(pay, [], "EUR")).toEqual({
      title: "Approve this payment?",
      message:
        "Example helper may then do it once for 42.00 EUR, to example.com plan, exactly as shown. Anything changed after this needs a new OK.",
    });
    expect(approveQuestion(approval(2)).title).toBe("Approve this post?");
  });
});

describe("what would hide in a request's words", () => {
  it("writes out bidi controls, zero-width and other format characters as [U+XXXX]", () => {
    expect(visible("example.com/\u202Egol")).toBe("example.com/[U+202E]gol");
    expect(visible("a\u2066b\u2069c\u200E\u200F")).toBe("a[U+2066]b[U+2069]c[U+200E][U+200F]");
    expect(visible("zero\u200B\u200C\u200D\u2060\uFEFFwidth")).toBe("zero[U+200B][U+200C][U+200D][U+2060][U+FEFF]width");
    expect(visible("tag\u{E0041}")).toBe("tag[U+E0041]");
  });

  it("writes out controls but a line break and a tab, and the line and paragraph separators", () => {
    expect(visible("one\ntwo\tthree")).toBe("one\ntwo\tthree");
    expect(visible("bell\u0007\r\n")).toBe("bell[U+0007][U+000D]\n");
    expect(visible("a\u0085b\u2028c")).toBe("a[U+0085]b[U+2028]c");
  });

  it("keeps everything else exactly as it is", () => {
    const text = "  Two spaces, é, مرحبا, 日本, 👍🏽 and a soft\u00A0space.";
    expect(visible(text)).toBe(text);
  });

  it("is used for the agent, the target, a file's name and who decided", () => {
    const tricky = approval(7, {
      agentName: "Help\u200Ber",
      target: "example.com/\u202Egol",
      approver: "lead",
      approverName: "Le\u2066ad",
    });
    expect(agentName(tricky)).toBe("Help[U+200B]er");
    expect(approverRole(tricky)).toBe("Le[U+2066]ad, the approver");
    expect(approveQuestion(tricky).message).toContain("to example.com/[U+202E]gol, exactly as shown");
    expect(fileLine({ name: "invoice\u202Efdp.exe", size: null, sha256: "ab".repeat(32) })).toBe(
      "invoice[U+202E]fdp.exe · size unknown · abababababab",
    );
    expect(agentLine(agent("client-a", { principal: "Ex\u200Dample" }))).toBe("Works for Ex[U+200D]ample");
  });
});

describe("who waits", () => {
  it("splits the pending requests into yours and others', in the server's order, leaving decided ones out", () => {
    const list = [
      approval(5),
      approval(4, { approver: "example-lead", waitsOnOwner: false }),
      approval(3, { state: "approved" }),
      approval(2, { kind: "payment" }),
      // The owner's kind, but the server says it doesn't wait on you (a disabled agent's): shown, not decided.
      approval(1, { waitsOnOwner: false }),
    ];
    const { yours, others } = approvalGroups(list);
    expect(yours.map((a) => a.id)).toEqual([5, 2]);
    expect(others.map((a) => a.id)).toEqual([4, 1]);
  });

  it("counts what waits on you from the state, nothing without approvals", () => {
    const state = {
      approvals: [approval(1), approval(2, { waitsOnOwner: false }), approval(3, { kind: "secret" })],
    } as unknown as EmberhutState;
    expect(approvalsWaitingOnYou(state)).toBe(2);
    expect(approvalsWaitingOnYou({} as EmberhutState)).toBe(0);
    expect(approvalsWaitingOnYou(undefined)).toBe(0);
  });

  it("sums them up for Needs you's row", () => {
    expect(approvalsSummary([approval(1), approval(2, { kind: "payment" }), approval(3, { waitsOnOwner: false })])).toEqual({
      yours: 2,
      title: "2 wait on you",
      subtitle: "1 only you can approve · 1 on others",
    });
    expect(approvalsSummary([approval(1)])).toEqual({ yours: 1, title: "1 waits on you", subtitle: null });
    expect(approvalsSummary([approval(3, { waitsOnOwner: false })])).toEqual({
      yours: 0,
      title: "None wait on you",
      subtitle: "1 on others",
    });
  });
});

describe("an agent's room", () => {
  it("counts only its requests that wait on you, as every other count does", () => {
    const state = {
      approvals: [approval(1), approval(2, { waitsOnOwner: false, approver: "example-lead" }), approval(3, { agent: "client-b" })],
    } as unknown as EmberhutState;
    expect(agentWaitingOnYou(state, "client-a")).toBe(1);
    expect(agentWaitingOnYou(undefined, "client-a")).toBe(0);
    expect(waitingWords(0)).toBe("No approvals wait on you");
    expect(waitingWords(1)).toBe("1 waits on you");
    expect(waitingWords(2)).toBe("2 wait on you");
  });

  it("writes out what hides in an unknown kind", () => {
    expect(kindName("re\u202Efund")).toBe("re[U+202E]fund");
  });
});

describe("agents", () => {
  it("are found by room, the default agent left out (its room is Main)", () => {
    const byRoom = agentsByRoom([agent("main", { room: 1 }), agent("client-a", { room: 9 }), agent("client-b", { room: 12 })]);
    expect([...byRoom.keys()]).toEqual([9, 12]);
    expect(byRoom.get(9)?.id).toBe("client-a");
    expect(agentsByRoom(undefined).size).toBe(0);
  });

  it("say who they work for, and Off while disabled", () => {
    expect(agentLine(agent("client-a"))).toBe("Works for Example Ltd");
    expect(agentLine(agent("client-a", { enabled: false }))).toBe("Works for Example Ltd · Off");
    expect(agentCaption(agent("client-a"), "Working…")).toBe("Working… · works for Example Ltd");
    expect(agentCaption(agent("client-a", { enabled: false }), "Watching")).toBe("Off · works for Example Ltd");
  });
});

describe("a decision that didn't go through", () => {
  it("says a 409 plainly and asks for the request again", () => {
    const failure = decisionFailure(new ApiError(409, "the payload changed"));
    expect(failure.refresh).toBe(true);
    expect(failure.message).toMatch(/changed or was already decided/);
  });

  it("tells a gone request, an unreachable server and any other refusal apart", () => {
    expect(decisionFailure(new ApiError(404, "no such request"))).toEqual({ message: "This request is gone.", refresh: true });
    expect(decisionFailure(new ApiError(0, "Can't reach Emberhut"))).toEqual({
      message: "Couldn't reach Emberhut: it may or may not have gone through. Here it is as the server has it.",
      refresh: true,
    });
    expect(decisionFailure(new ApiError(400, "payloadHash: not 64 hex digits"))).toEqual({
      message: "Not done: payloadHash: not 64 hex digits",
      refresh: true,
    });
    expect(decisionFailure(new Error("boom"))).toEqual({ message: "Not done: boom", refresh: false });
  });
});
