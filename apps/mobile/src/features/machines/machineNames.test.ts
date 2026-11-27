import { describe, expect, it } from "vitest";
import type { Machine, MachinesPage } from "@/api";
import { machineLabel, machineNamesOf, noMachineNames, placeableMachines } from "./machineNames";

const machine = (id: string, name: string, placeable = true) => ({ id, name, placeable }) as Machine;
const page = {
  machines: [machine("machine-b", "Build box"), machine("machine-a", "Studio", false), machine("machine-c", " ")],
} as MachinesPage;

describe("machine names", () => {
  it("names a machine as the server does, else by its id", () => {
    const names = machineNamesOf(page);
    expect(machineLabel(names, "machine-b")).toBe("Build box");
    expect(machineLabel(names, "machine-c")).toBe("machine-c");
    expect(machineLabel(names, "elsewhere")).toBe("elsewhere");
    expect(machineLabel(noMachineNames, "machine-b")).toBe("machine-b");
    expect([...names.keys()]).toEqual(["machine-b", "machine-a", "machine-c"]);
  });

  it("offers a new session only the machines the server places work on, in its order", () => {
    expect(placeableMachines(page)).toEqual([
      { id: "machine-b", name: "Build box" },
      { id: "machine-c", name: "machine-c" },
    ]);
    expect(placeableMachines(undefined)).toEqual([]);
  });
});
