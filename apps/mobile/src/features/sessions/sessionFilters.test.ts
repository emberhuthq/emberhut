import { describe, expect, it } from "vitest";
import type { AgentNow, RunnerSession } from "@/api";
import {
  EXTERNAL_THREAD_KIND,
  agentFacts,
  chipLabel,
  keeps,
  sessionFacts,
  sessionPicks,
  externalThreadFacts,
  toggle,
  type SessionFacts,
} from "./sessionFilters";

const facts = (kind: string, machine: string | null, account: string | null): SessionFacts => ({ kind, machine, account });
const rows = [
  facts("worker", "machine-a", "Personal Claude"),
  facts("helper", "machine-a", "Personal Claude"),
  facts("helper", "machine-a", "Personal Claude"),
  facts("lead", "machine-a", "AWS Bedrock"),
  facts("session", "machine-b", "AWS Bedrock"),
  facts("direct", "machine-c", "AWS Bedrock"),
];

describe("session filters", () => {
  it("reads who runs a row and where", () => {
    const a = { lead: true, name: "Worker", kind: null, machine: null, account: null, accountName: null } as unknown as AgentNow;
    expect(agentFacts(a)).toEqual(facts("worker", null, null)); // an older server: the agent's name says it
    expect(agentFacts({ ...a, lead: false, name: "Part 1" })).toEqual(facts("helper", null, null));
    expect(agentFacts({ ...a, kind: "lead", machine: "machine-b", account: "bedrock", accountName: "AWS Bedrock" })).toEqual(
      facts("lead", "machine-b", "AWS Bedrock"),
    );
    const s = { direct: true, machine: "machine-c", account: "team", accountName: "Claude Team" } as RunnerSession;
    expect(sessionFacts(s)).toEqual(facts("direct", "machine-c", "Claude Team"));
    // An external thread keeps its own machine id: the app maps no machine onto another.
    expect(externalThreadFacts({ machine: "external-b" } as never)).toEqual(facts(EXTERNAL_THREAD_KIND, "external-b", null));
  });

  it("adds picks up in one filter and narrows across them", () => {
    expect(rows.filter((x) => keeps({}, x))).toHaveLength(6);
    expect(rows.filter((x) => keeps({ workers: ["worker", "helper"] }, x))).toHaveLength(3);
    expect(rows.filter((x) => keeps({ machines: ["machine-b", "machine-c"] }, x))).toHaveLength(2);
    expect(rows.filter((x) => keeps({ machines: ["machine-a"], accounts: ["AWS Bedrock"] }, x))).toEqual([rows[3]]);
  });

  it("counts each pick with the other filters as they are", () => {
    const names = new Map([
      ["machine-c", "Workbench"],
      ["machine-a", "Studio"],
    ]);
    const p = sessionPicks({ accounts: ["AWS Bedrock"] }, rows, names);
    expect(p.workers.map((x) => [x.key, x.count])).toEqual([
      ["session", 1],
      ["lead", 1],
      ["direct", 1],
    ]);
    // The server's machines in its order and by its names, then one it doesn't list, by id.
    expect(p.machines.map((x) => [x.label, x.count])).toEqual([
      ["Workbench", 1],
      ["Studio", 1],
      ["machine-b", 1],
    ]);
    expect(sessionPicks({}, rows).machines.map((x) => x.label)).toEqual(["machine-a", "machine-b", "machine-c"]);
    // The account menu ignores its own pick: both accounts keep their full counts.
    expect(p.accounts.map((x) => [x.key, x.count])).toEqual([
      ["AWS Bedrock", 3],
      ["Personal Claude", 3],
    ]);
    // A pick that no row has any more stays offered (at 0), so it can be taken off.
    expect(sessionPicks({ workers: [EXTERNAL_THREAD_KIND] }, rows).workers.find((x) => x.key === EXTERNAL_THREAD_KIND)?.count).toBe(0);
  });

  it("toggles picks and names the chip", () => {
    expect(toggle(null, "mac")).toEqual(["mac"]);
    expect(toggle(["mac"], "mac")).toBeNull();
    expect(chipLabel("Worker", [])).toBe("Worker");
    expect(
      chipLabel("Worker", [
        { key: "a", label: "Lead", count: 1 },
        { key: "b", label: "Worker", count: 2 },
      ]),
    ).toBe("Lead, Worker");
    expect(
      chipLabel(
        "Env",
        [1, 2, 3].map((i) => ({ key: `${i}`, label: `M${i}`, count: i })),
      ),
    ).toBe("M1 +2");
  });
});
