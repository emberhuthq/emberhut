import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { FactsPage, SettingsChange } from "@/api";
import { api } from "@/services";
import { queryKeys } from "@/state/query";
import { CantKeepEarly, stillOnTrial, withChange } from "./facts";

/**
 * A fix, a forget, a person's wake switch, or a kept or dropped lesson: shown at once, sent, then the page asked for again.
 * A server older than `trialKeep` answers a keep but leaves the lesson on trial: the page read after shows it, and the
 * change fails with CantKeepEarly so the lesson comes back with a plain line.
 */
export function useFactChange() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (change: SettingsChange) => {
      await api.settings.apply(change);
      if (change.trialKeep !== undefined && stillOnTrial(await api.settings.facts(), change.trialKeep)) throw new CantKeepEarly();
    },
    onMutate: async (change) => {
      await client.cancelQueries({ queryKey: queryKeys.facts });
      const before = client.getQueryData<FactsPage>(queryKeys.facts);
      if (before) client.setQueryData(queryKeys.facts, withChange(before, change));
      return { before };
    },
    onError: (_error, _change, context) => {
      if (context?.before) client.setQueryData(queryKeys.facts, context.before);
    },
    onSettled: () => client.invalidateQueries({ queryKey: queryKeys.facts }),
  });
}
