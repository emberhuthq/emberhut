import { useIsFocused } from "@react-navigation/native";
import { inBackground } from "@/log";
import { useMachines } from "@/state/queries";
import { EmptyState, ErrorState, Loading, Screen, Text, usePullRefresh } from "@/ui";
import { MachineCard } from "./MachineCard";
import { isOlderServer, olderServerText } from "./machines";

/** Each runner machine's live capacity, as the server lists them, asked every 10 s while the page is in front. */
export function MachinesScreen() {
  const focused = useIsFocused();
  const page = useMachines(focused);
  const pull = usePullRefresh(page.refetch);
  if (page.isPending) return <Loading label="Asking the machines…" />;
  if (!page.data) {
    if (isOlderServer(page.error)) return <EmptyState icon="hardware-chip-outline" title="Not yet" message={olderServerText} />;
    return (
      <ErrorState
        message={page.error?.message ?? "Can't read the machines"}
        onRetry={() => inBackground(page.refetch(), "MachinesScreen: refetch")}
      />
    );
  }
  return (
    <Screen scroll {...pull}>
      {page.isError ? (
        <Text variant="footnote" tone="warning">
          {`Can't refresh: ${page.error.message}`}
        </Text>
      ) : null}
      {page.data.machines.map((m) => (
        <MachineCard key={m.id} machine={m} limits={page.data.thresholds} />
      ))}
    </Screen>
  );
}
