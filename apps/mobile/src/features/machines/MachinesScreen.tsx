import { useIsFocused, useNavigation } from "@react-navigation/native";
import { Pressable, View } from "react-native";
import { inBackground } from "@/log";
import { useEmberhutState, useMachines } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, EmptyState, ErrorState, Icon, Loading, Screen, Text, usePullRefresh } from "@/ui";
import { accountsOn, countLine, updatedLine } from "./huts";
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
  const { colors, radius, spacing } = useTheme();
  const pull = usePullRefresh(page.refetch);
  // What "Add a hut" does, from the button and the dashed row alike: the pairing code in Settings › Devices.
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
        <View style={{ flex: 1 }}>
          <Text variant="footnote" tone="secondary">
            {page.data.machines.length === 0 ? "No huts have reported yet" : countLine(page.data.machines)}
          </Text>
          <Text variant="caption" tone="secondary">
            {updatedLine(new Date(page.dataUpdatedAt))}
          </Text>
        </View>
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
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Add a hut: pair another computer"
        onPress={addHut}
        style={({ pressed }) => ({
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "center",
          gap: spacing.xs,
          paddingVertical: spacing.md,
          borderRadius: radius.lg,
          borderWidth: 1,
          borderStyle: "dashed",
          borderColor: colors.border,
          opacity: pressed ? 0.6 : 1,
        })}
      >
        <Icon name="add-circle-outline" size={18} color={colors.textSecondary} />
        <Text tone="secondary">Add a hut</Text>
      </Pressable>
    </Screen>
  );
}
