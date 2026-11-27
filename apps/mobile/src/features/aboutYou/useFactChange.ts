import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { FactsPage, SettingsChange } from "@/api";
import { api } from "@/services";
import { queryKeys } from "@/state/query";
import { withChange } from "./facts";

/** A fix, a forget, a person's wake switch or a dropped rule: shown at once, sent, then the page asked for again. */
export function useFactChange() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (change: SettingsChange) => api.settings.apply(change),
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
