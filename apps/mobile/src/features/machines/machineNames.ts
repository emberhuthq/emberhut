import type { MachinesPage } from "@/api";

// What the app calls a runner machine: the name the server gives it (GET /api/machines), never one the app makes up.
// A machine the server doesn't list (an older server, an external thread's) is shown by its id.

/** Each machine's name by its id, in the server's order. */
export type MachineNames = ReadonlyMap<string, string>;

export const noMachineNames: MachineNames = new Map();

export const machineNamesOf = (page: MachinesPage | undefined): MachineNames =>
  new Map((page?.machines ?? []).map((m) => [m.id, m.name.trim() || m.id]));

/** A machine's name, else its id. */
export const machineLabel = (names: MachineNames, id: string): string => names.get(id) ?? id;

/** The machines a new session can go to, in the server's order: those it places work on. */
export const placeableMachines = (page: MachinesPage | undefined): { id: string; name: string }[] =>
  (page?.machines ?? []).filter((m) => m.placeable).map((m) => ({ id: m.id, name: m.name.trim() || m.id }));
