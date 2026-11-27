import { useIsFocused } from "@react-navigation/native";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useLayoutEffect, useState } from "react";
import { Alert, Pressable, StyleSheet, View } from "react-native";
import type { Party } from "@/api";
import { isOlderServer } from "@/features/machines/machines";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { api } from "@/services";
import { useParties } from "@/state/queries";
import { queryKeys } from "@/state/query";
import { useTheme } from "@/theme";
import { Button, EmptyState, ErrorState, Icon, Loading, Screen, Section, Text, usePullRefresh } from "@/ui";
import { castLine, olderServerText, partyCost, statusWord } from "./party";
import { CastDots, StatusDot } from "./PartyParts";

/** Your parties, newest first: each with its cast, state, last line and cost; + starts a new one, a long press deletes one. */
export function PartiesScreen({ navigation }: RootScreenProps<"Parties">) {
  const { colors, spacing } = useTheme();
  const client = useQueryClient();
  const focused = useIsFocused();
  const page = useParties(focused);
  const pull = usePullRefresh(page.refetch);
  const [gone, setGone] = useState<number[]>([]);
  const [problem, setProblem] = useState<string | null>(null);
  const { refetch } = page;

  // Back from a party, its new lines and cost show at once.
  useEffect(() => {
    if (focused) inBackground(refetch(), "PartiesScreen: refetch");
  }, [focused, refetch]);

  useLayoutEffect(() => {
    navigation.setOptions({
      headerRight: () => (
        <Pressable onPress={() => navigation.navigate("NewParty")} accessibilityRole="button" accessibilityLabel="New party" hitSlop={8}>
          <Icon name="add" size={26} color={colors.text} />
        </Pressable>
      ),
    });
  }, [navigation, colors.text]);

  const remove = (party: Party) =>
    Alert.alert("Delete this party?", `"${party.title}" and all its talk are gone for good.`, [
      { text: "Cancel", style: "cancel" },
      {
        text: "Delete",
        style: "destructive",
        onPress: () => {
          setGone((g) => [...g, party.id]);
          api.parties
            .delete(party.id)
            .then(() => setProblem(null))
            .catch((cause: unknown) => {
              setGone((g) => g.filter((id) => id !== party.id));
              setProblem(`Not deleted: ${cause instanceof Error ? cause.message : "unknown"}`);
            })
            .finally(() => inBackground(client.invalidateQueries({ queryKey: queryKeys.parties }), "PartiesScreen: invalidateQueries"));
        },
      },
    ]);

  if (page.isPending) return <Loading label="Looking…" />;
  if (!page.data) {
    if (isOlderServer(page.error)) return <EmptyState icon="people-outline" title="Not yet" message={olderServerText} />;
    return (
      <ErrorState
        message={page.error?.message ?? "Couldn't read your parties"}
        onRetry={() => inBackground(refetch(), "PartiesScreen: refetch")}
      />
    );
  }
  const parties = page.data.parties.filter((p) => !gone.includes(p.id));
  if (parties.length === 0)
    return (
      <EmptyState
        icon="people-outline"
        title="AI personas talking to each other"
        message="Pick 2 to 5 personas, give them a topic, and watch them talk it out. Jump in, steer them or stop them any time."
        action={<Button label="New party" icon="add" onPress={() => navigation.navigate("NewParty")} />}
      />
    );
  return (
    <Screen scroll {...pull}>
      {page.isError ? (
        <Text variant="footnote" tone="warning">
          {`Can't refresh: ${page.error.message}`}
        </Text>
      ) : null}
      {problem ? (
        <Text variant="footnote" tone="danger">
          {problem}
        </Text>
      ) : null}
      <Section>
        {parties.map((p) => (
          <Pressable
            key={p.id}
            onPress={() => navigation.navigate("Party", { id: p.id, title: p.title })}
            onLongPress={() => remove(p)}
            accessibilityRole="button"
            accessibilityHint="Long press to delete it"
            style={({ pressed }) => [
              styles.row,
              {
                gap: spacing.md,
                paddingHorizontal: spacing.lg,
                paddingVertical: spacing.md,
                backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
              },
            ]}
          >
            <View style={{ paddingTop: 2 }}>
              <CastDots personas={p.personas} />
            </View>
            <View style={styles.fill}>
              <View style={[styles.line, { gap: spacing.sm }]}>
                <Text weight="500" numberOfLines={1} style={styles.fill}>
                  {p.title}
                </Text>
                <Text variant="caption" tone="secondary" style={styles.digits}>
                  {partyCost(p.cost)}
                </Text>
              </View>
              <View style={[styles.line, { gap: spacing.xs + 1 }]}>
                <StatusDot status={p.status} />
                <Text variant="caption" tone="secondary" numberOfLines={1} style={styles.fill}>
                  {`${statusWord(p.status)} · ${castLine(p)}`}
                </Text>
              </View>
              {p.last ? (
                <Text variant="callout" tone="secondary" numberOfLines={2}>
                  {p.last}
                </Text>
              ) : null}
            </View>
          </Pressable>
        ))}
      </Section>
    </Screen>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "flex-start" },
  fill: { flex: 1, gap: 3 },
  line: { flexDirection: "row", alignItems: "center" },
  digits: { fontVariant: ["tabular-nums"] },
});
