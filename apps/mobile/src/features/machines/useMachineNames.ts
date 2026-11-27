import { useMachineSummary } from "@/state/queries";
import { machineNamesOf, type MachineNames } from "./machineNames";

/** The server's machine names, from the machines without their history (asked again after 5 minutes at most). */
export function useMachineNames(): MachineNames {
  return machineNamesOf(useMachineSummary().data);
}
