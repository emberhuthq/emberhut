import { useCallback, useState } from "react";
import { inBackground } from "@/log";

/**
 * Pull to refresh: the spinner shows only while a pull's own refetch runs, never for the background polls
 * (a query's isRefetching is true on every one of them).
 */
export function usePullRefresh(refetch: () => Promise<unknown>): { refreshing: boolean; onRefresh: () => void } {
  const [refreshing, setRefreshing] = useState(false);
  const onRefresh = useCallback(() => {
    setRefreshing(true);
    inBackground(
      refetch().finally(() => setRefreshing(false)),
      "usePullRefresh: refetch",
    );
  }, [refetch]);
  return { refreshing, onRefresh };
}
