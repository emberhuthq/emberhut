import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Alert, Pressable, View } from "react-native";
import type { Fact, TrialRule } from "@/api";
import { parseInstant, shortDate } from "@/format/time";
import { inBackground } from "@/log";
import { useFacts } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, EmptyState, ErrorState, Icon, Loading, Screen, Section, Text, TextField, usePullRefresh } from "@/ui";
import { factGroups, trialsShown } from "./facts";
import { useFactChange } from "./useFactChange";

/**
 * What Emberhut keeps about you, one short fact per line, grouped by category, each with where it came from; a tap fixes or
 * forgets one. On top, the rules Emberhut is trying out from your corrections, each droppable.
 */
export function AboutYouScreen() {
  const facts = useFacts();
  const pull = usePullRefresh(facts.refetch);
  const [query, setQuery] = useState("");
  const change = useFactChange();
  if (facts.isPending) return <Loading />;
  if (facts.isError)
    return <ErrorState message={facts.error.message} onRetry={() => inBackground(facts.refetch(), "AboutYouScreen: refetch")} />;
  const groups = factGroups(facts.data.facts, query);
  const trials = trialsShown(facts.data.trials ?? [], query);
  const drop = (rule: TrialRule) =>
    Alert.alert("Drop this rule?", `Emberhut stops following "${rule.text}"`, [
      { text: "Cancel", style: "cancel" },
      { text: "Drop", style: "destructive", onPress: () => change.mutate({ trialDrop: rule.id }) },
    ]);
  return (
    <Screen scroll {...pull}>
      <TextField
        value={query}
        onChangeText={setQuery}
        placeholder="Search what Emberhut knows"
        accessibilityLabel="Search what Emberhut knows"
        autoCorrect={false}
      />
      {facts.data.facts.length === 0 && trials.length === 0 ? (
        <EmptyState
          icon="person-circle-outline"
          title="Nothing yet"
          message="As Emberhut learns about you, short facts show up here, and you can fix or forget any of them."
        />
      ) : null}
      {change.isError ? (
        <Text variant="footnote" tone="danger">
          Not saved: {change.error.message}
        </Text>
      ) : null}
      <Section
        title={`Emberhut is trying out · ${trials.length}`}
        footer="Lessons from your corrections. Emberhut follows each one for 14 days; Saturday's review keeps or drops it."
      >
        {trials.map((rule) => (
          <TrialRow key={rule.id} rule={rule} onDrop={() => drop(rule)} />
        ))}
      </Section>
      {groups.map((group) => (
        <Section key={group.key} title={`${group.name} · ${group.facts.length}`}>
          {group.facts.map((fact) => (
            <FactRow key={fact.id} fact={fact} />
          ))}
        </Section>
      ))}
      {groups.length === 0 && trials.length === 0 && query.trim() !== "" ? (
        <Text tone="secondary">Nothing matches “{query.trim()}”.</Text>
      ) : null}
    </Screen>
  );
}

function TrialRow({ rule, onDrop }: { rule: TrialRule; onDrop: () => void }) {
  const { colors, spacing } = useTheme();
  const from = parseInstant(rule.at);
  const to = parseInstant(rule.ends);
  return (
    <View style={{ flexDirection: "row", gap: spacing.md, padding: spacing.lg, backgroundColor: colors.surface, alignItems: "flex-start" }}>
      <View style={{ flex: 1, gap: spacing.xs }}>
        <Text>{rule.text}</Text>
        {rule.said ? (
          <Text variant="footnote" tone="secondary" numberOfLines={2}>
            You said: {rule.said}
          </Text>
        ) : null}
        <Text variant="caption" tone="tertiary">
          From your correction on {from ? shortDate(from) : "?"} · on trial until {to ? shortDate(to) : "?"}
        </Text>
      </View>
      <Button label="Drop" variant="destructive" onPress={onDrop} accessibilityLabel={`Drop the rule: ${rule.text}`} />
    </View>
  );
}

function FactRow({ fact }: { fact: Fact }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const updated = parseInstant(fact.updated);
  const meta = [fact.source, updated ? shortDate(updated) : null].filter(Boolean).join(" · ");
  return (
    <Pressable
      onPress={() => navigation.navigate("Fact", { id: fact.id })}
      accessibilityRole="button"
      accessibilityLabel={`${fact.title}: ${fact.text}`}
      accessibilityHint="Fix or forget it"
      style={({ pressed }) => ({ padding: spacing.lg, gap: spacing.xs, backgroundColor: pressed ? colors.surfaceRaised : colors.surface })}
    >
      <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.xs }}>
        <Text variant="callout" weight="600">
          {fact.title}
        </Text>
        {fact.important ? <Icon name="notifications" size={13} color={colors.warning} accessibilityLabel="Wakes Emberhut" /> : null}
      </View>
      {fact.text ? <Text>{fact.text}</Text> : null}
      {fact.now ? (
        <Text variant="footnote" tone="secondary">
          Now: {fact.now}
        </Text>
      ) : null}
      {meta ? (
        <Text variant="caption" tone="tertiary">
          {meta}
        </Text>
      ) : null}
    </Pressable>
  );
}
