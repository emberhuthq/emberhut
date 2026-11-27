import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Alert, Pressable, ScrollView, View } from "react-native";
import type { Fact, TrialRule } from "@/api";
import { parseInstant, shortDate } from "@/format/time";
import { inBackground } from "@/log";
import { useFacts } from "@/state/queries";
import { minTap, useTheme } from "@/theme";
import { EmptyState, ErrorState, Icon, type IconName, Loading, Screen, Section, Text, TextField, usePullRefresh } from "@/ui";
import {
  CantKeepEarly,
  type CategoryChip,
  changeProblem,
  categoryChips,
  decideBy,
  factGroups,
  groupsShown,
  lessonText,
  lessonsToReview,
  searchPlaceholder,
  trialsShown,
} from "./facts";
import { useLearningFold } from "./learningFold";
import { useFactChange } from "./useFactChange";

/**
 * What your ember knows about you. Under the search, "Learning from you" folded into one line: the lessons from your
 * corrections, each to keep or drop before the weekly review decides. Then the facts by category, with a chip per category
 * to narrow them; a tap (or the pencil) fixes a fact, the bin forgets it after asking once.
 */
export function AboutYouScreen() {
  const facts = useFacts();
  const pull = usePullRefresh(facts.refetch);
  const [query, setQuery] = useState("");
  const [chip, setChip] = useState<string | null>(null);
  const open = useLearningFold((s) => s.open);
  const toggle = useLearningFold((s) => s.toggle);
  const change = useFactChange();
  if (facts.isPending) return <Loading />;
  if (facts.isError)
    return <ErrorState message={facts.error.message} onRetry={() => inBackground(facts.refetch(), "AboutYouScreen: refetch")} />;
  const groups = factGroups(facts.data.facts, query);
  const chips = categoryChips(facts.data.facts, query);
  const shown = groupsShown(groups, chip);
  const active = shown.length === groups.length ? null : chip;
  const trials = trialsShown(facts.data.trials ?? [], query);
  const forget = (fact: Fact) =>
    Alert.alert(`Forget "${fact.title}"?`, "Your ember stops using it.", [
      { text: "Cancel", style: "cancel" },
      { text: "Forget", style: "destructive", onPress: () => change.mutate({ factDrop: fact.id }) },
    ]);
  const drop = (rule: TrialRule) =>
    Alert.alert("Drop this lesson?", "Your ember stops following it.", [
      { text: "Cancel", style: "cancel" },
      { text: "Drop", style: "destructive", onPress: () => change.mutate({ trialDrop: rule.id }) },
    ]);
  const nothingYet = groups.length === 0 && (facts.data.trials ?? []).length === 0 && query.trim() === "";
  return (
    <Screen scroll {...pull}>
      <TextField
        value={query}
        onChangeText={setQuery}
        placeholder={searchPlaceholder}
        accessibilityLabel={searchPlaceholder}
        autoCorrect={false}
      />
      {nothingYet ? (
        <EmptyState
          icon="person-circle-outline"
          title="Nothing yet"
          message="As your ember learns about you, short facts show up here, and you can fix or forget any of them."
        />
      ) : null}
      {change.isError ? (
        <Text variant="footnote" tone={change.error instanceof CantKeepEarly ? "secondary" : "danger"}>
          {changeProblem(change.error)}
        </Text>
      ) : null}
      {trials.length > 0 ? (
        <LearningFromYou
          trials={trials}
          open={open}
          onToggle={toggle}
          onKeep={(rule) => change.mutate({ trialKeep: rule.id })}
          onDrop={drop}
        />
      ) : null}
      {groups.length > 0 ? <Chips chips={chips} active={active} onChoose={setChip} /> : null}
      {shown.map((group) => (
        <Section key={group.key} title={group.name} count={group.facts.length}>
          {group.facts.map((fact) => (
            <FactRow key={fact.id} fact={fact} onForget={() => forget(fact)} />
          ))}
        </Section>
      ))}
      {groups.length === 0 && trials.length === 0 && query.trim() !== "" ? (
        <Text tone="secondary">Nothing matches “{query.trim()}”.</Text>
      ) : null}
    </Screen>
  );
}

interface LearningProps {
  trials: TrialRule[];
  open: boolean;
  onToggle: () => void;
  onKeep: (rule: TrialRule) => void;
  onDrop: (rule: TrialRule) => void;
}

/** "Learning from you": one line with how many lessons wait and the day to decide by, opening in place to each lesson. */
function LearningFromYou({ trials, open, onToggle, onKeep, onDrop }: LearningProps) {
  const { colors, radius, spacing } = useTheme();
  const by = decideBy(trials);
  const byDay = by ? parseInstant(by) : null;
  const summary = [lessonsToReview(trials.length), byDay ? `decide by ${shortDate(byDay)}` : null].filter(Boolean).join(" · ");
  return (
    <View style={{ backgroundColor: colors.surface, borderRadius: radius.lg, overflow: "hidden" }}>
      <Pressable
        onPress={onToggle}
        accessibilityRole="button"
        accessibilityLabel={`Learning from you, ${summary}`}
        accessibilityState={{ expanded: open }}
        style={({ pressed }) => ({
          minHeight: minTap,
          flexDirection: "row",
          alignItems: "center",
          gap: spacing.sm,
          paddingHorizontal: spacing.lg,
          paddingVertical: spacing.sm,
          backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
        })}
      >
        <Icon name={open ? "chevron-down" : "chevron-forward"} size={16} color={colors.textTertiary} />
        <View style={{ flex: 1 }}>
          <Text variant="callout" weight="600">
            Learning from you
          </Text>
          <Text variant="footnote" tone="secondary">
            {summary}
          </Text>
        </View>
      </Pressable>
      {open
        ? trials.map((rule) => (
            <View key={rule.id} style={{ borderTopWidth: 1, borderTopColor: colors.border }}>
              <LessonRow rule={rule} onKeep={() => onKeep(rule)} onDrop={() => onDrop(rule)} />
            </View>
          ))
        : null}
    </View>
  );
}

/** A lesson: your own words as a quote, the lesson in plain words, until when it's on trial, Keep and Drop. */
function LessonRow({ rule, onKeep, onDrop }: { rule: TrialRule; onKeep: () => void; onDrop: () => void }) {
  const { colors, radius, spacing } = useTheme();
  const ends = parseInstant(rule.ends);
  const said = rule.said.trim();
  const lesson = lessonText(rule);
  return (
    <View style={{ padding: spacing.lg, gap: spacing.sm }}>
      {said ? (
        <View style={{ borderLeftWidth: 2, borderLeftColor: colors.accent, paddingLeft: spacing.md }}>
          <Text style={{ fontStyle: "italic" }}>“{said}”</Text>
        </View>
      ) : null}
      <Text tone={said ? "secondary" : "primary"}>{lesson}</Text>
      <Text variant="caption" tone="tertiary">
        On trial until {ends ? shortDate(ends) : "?"}. If you don&rsquo;t choose, the weekly review decides.
      </Text>
      <View style={{ flexDirection: "row", gap: spacing.sm }}>
        {(
          [
            ["Keep", onKeep, "primary", `Keep the lesson: ${lesson}`],
            ["Drop", onDrop, "danger", `Drop the lesson: ${lesson}`],
          ] as const
        ).map(([label, onPress, tone, a11y]) => (
          <Pressable
            key={label}
            onPress={onPress}
            accessibilityRole="button"
            accessibilityLabel={a11y}
            style={({ pressed }) => ({
              minHeight: minTap,
              paddingHorizontal: spacing.lg,
              justifyContent: "center",
              borderRadius: radius.md,
              borderWidth: 1,
              borderColor: colors.border,
              backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
            })}
          >
            <Text variant="callout" weight="600" tone={tone}>
              {label}
            </Text>
          </Pressable>
        ))}
      </View>
    </View>
  );
}

/** The category chips: All, then each category and how many; the chosen one filled. */
function Chips({ chips, active, onChoose }: { chips: CategoryChip[]; active: string | null; onChoose: (key: string | null) => void }) {
  const { colors, radius, spacing } = useTheme();
  return (
    <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ gap: spacing.sm }}>
      {chips.map((chip) => {
        const chosen = chip.key === active;
        return (
          <Pressable
            key={chip.key ?? "all"}
            onPress={() => onChoose(chip.key)}
            accessibilityRole="button"
            accessibilityLabel={`${chip.label}: ${chip.count} facts`}
            accessibilityState={{ selected: chosen }}
            hitSlop={{ top: 6, bottom: 6 }}
            style={{
              minHeight: 32,
              justifyContent: "center",
              paddingHorizontal: spacing.md,
              borderRadius: radius.pill,
              borderWidth: 1,
              borderColor: chosen ? colors.primary : colors.border,
              backgroundColor: chosen ? colors.primary : colors.surface,
            }}
          >
            <Text variant="footnote" weight="600" style={{ color: chosen ? colors.onPrimary : colors.textSecondary }}>
              {chip.key === null ? chip.label : `${chip.label} ${chip.count}`}
            </Text>
          </Pressable>
        );
      })}
    </ScrollView>
  );
}

/** A small square button with an icon, as tall as a tap needs. */
function RowButton({ icon, label, onPress, color }: { icon: IconName; label: string; onPress: () => void; color: string }) {
  const { colors, radius } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={({ pressed }) => ({
        width: minTap,
        height: minTap,
        alignItems: "center",
        justifyContent: "center",
        borderRadius: radius.sm,
        backgroundColor: pressed ? colors.surfaceRaised : "transparent",
      })}
    >
      <Icon name={icon} size={18} color={color} />
    </Pressable>
  );
}

/** A fact: a tap or the pencil fixes it, the bin forgets it. */
function FactRow({ fact, onForget }: { fact: Fact; onForget: () => void }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const updated = parseInstant(fact.updated);
  const meta = [fact.source, updated ? shortDate(updated) : null].filter(Boolean).join(" · ");
  const edit = () => navigation.navigate("Fact", { id: fact.id });
  return (
    <View style={{ flexDirection: "row", alignItems: "flex-start", backgroundColor: colors.surface }}>
      <Pressable
        onPress={edit}
        accessibilityRole="button"
        accessibilityLabel={`${fact.title}: ${fact.text}`}
        accessibilityHint="Fix it"
        style={({ pressed }) => ({
          flex: 1,
          paddingVertical: spacing.lg,
          paddingLeft: spacing.lg,
          gap: spacing.xs,
          backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
        })}
      >
        <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.xs }}>
          <Text variant="callout" weight="600">
            {fact.title}
          </Text>
          {fact.important ? (
            <Icon name="notifications" size={13} color={colors.warning} accessibilityLabel="Tells you right away when they write" />
          ) : null}
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
      <View style={{ flexDirection: "row", paddingTop: spacing.sm, paddingRight: spacing.xs }}>
        <RowButton icon="create-outline" label={`Edit ${fact.title}`} onPress={edit} color={colors.textSecondary} />
        <RowButton icon="trash-outline" label={`Forget ${fact.title}`} onPress={onForget} color={colors.danger} />
      </View>
    </View>
  );
}
