import { useIsFocused, useNavigation } from "@react-navigation/native";
import { View } from "react-native";
import { inBackground } from "@/log";
import { useEmberhutState, useMachines } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, EmptyState, ErrorState, Loading, Screen, Text, usePullRefresh } from "@/ui";
import { accountsOn, countLine } from "./huts";
import { MachineCard } from "./MachineCard";
import { isOlderServer, olderServerText } from "./machines";

/**
 * Huts: each computer the assistant works on, as the server lists them (asked every 10 s while the page is in front), with
 * the accounts kept on each from the state. Add a hut opens the pairing code in Settings › Devices.
 */
export function MachinesScreen() {
  const focused = useIsFocused();
  const page = useMachines(focused);
  const state = useEmberhutState();
  const navigation = useNavigation();
  const { spacing } = useTheme();
  const pull = usePullRefresh(page.refetch);
  const addHut = () => navigation.navigate("SettingsGroup", { group: "devices" });
  if (page.isPending) return <Loading label="Asking the huts…" />;
  if (!page.data) {
    if (isOlderServer(page.error)) return <EmptyState icon="hardware-chip-outline" title="Not yet" message={olderServerText} />;
    return (
      <ErrorState
        message={page.error?.message ?? "Can't read the huts"}
        onRetry={() => inBackground(page.refetch(), "MachinesScreen: refetch")}
      />
    );
  }
  const accounts = state.data?.accounts;
  return (
    <Screen scroll {...pull}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.md }}>
        <Text variant="footnote" tone="secondary" style={{ flex: 1 }}>
          {page.data.machines.length === 0 ? "No huts have reported yet" : countLine(page.data.machines)}
        </Text>
        <Button label="Add a hut" icon="add" variant="plain" onPress={addHut} accessibilityLabel="Add a hut: pair another computer" />
      </View>
      {page.isError ? (
        <Text variant="footnote" tone="warning">
          {`Can't refresh: ${page.error.message}`}
        </Text>
      ) : null}
      {page.data.machines.map((m) => (
        <MachineCard key={m.id} machine={m} limits={page.data.thresholds} accounts={accountsOn(accounts, m)} />
      ))}
    </Screen>
  );
}
