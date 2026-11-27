import * as Clipboard from "expo-clipboard";
import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, View } from "react-native";
import type { Decision, DecisionChoice, TaskArtifact } from "@/api";
import { ServerImage } from "@/features/files/ServerImage";
import { answerLabel, askIsStale, askText } from "@/features/needsYou/model";
import { parseDay, parseInstant, relative, shortDate, weekday } from "@/format/time";
import { inBackground } from "@/log";
import { minTap, useTheme } from "@/theme";
import { Button, Card, Chip, Collapsible, Icon, Section, SectionHeading, Text } from "@/ui";

/** Why you're asked now and by when, under the action at the top of the page. */
export function DecisionContext({ decision, decideBy }: { decision: Decision; decideBy?: string | null }) {
  const by = parseDay(decideBy);
  const cards = (decision.choices ?? []).filter((c) => c.title).length;
  const line = [by ? `Decide by ${weekday(by)} ${shortDate(by)}` : null, cards > 1 ? `${cards} options to compare` : null].filter(Boolean);
  return (
    <>
      {decision.context?.trim() ? <Text variant="callout">{decision.context}</Text> : null}
      {line.length > 0 ? (
        <Text variant="footnote" tone="secondary">
          {line.join(" · ")}
        </Text>
      ) : null}
    </>
  );
}

/**
 * Laid out to decide at a glance: what you need to know (Emberhut's bullets, or the question), when it was asked and checked,
 * the draft to send, then each answer with what it does and Emberhut's pick, as cards when Emberhut made them.
 */
export function DecisionBlock({
  decision,
  updated,
  artifacts,
  onAnswer,
}: {
  decision: Decision;
  updated: string;
  artifacts: TaskArtifact[];
  onAnswer: (choice: string) => void;
}) {
  const { spacing } = useTheme();
  const text = askText(decision);
  const brief = decision.brief ?? [];
  const choices = decision.choices ?? [];
  const cards = choices.some((c) => c.title);
  const asked = parseInstant(decision.asked ?? updated);
  const checked = parseInstant(decision.checked ?? decision.asked ?? updated);
  const stale = askIsStale(decision, updated);
  return (
    <>
      <View style={{ gap: spacing.sm }}>
        <SectionHeading>What you need to know</SectionHeading>
        <Card>
          {brief.length === 0 ? <Collapsible text={text.summary} lines={cards ? 3 : 6} /> : null}
          {brief.map((line) => (
            <View key={line} style={{ flexDirection: "row", gap: spacing.sm }}>
              <Text tone="secondary">•</Text>
              <Text style={{ flex: 1 }} selectable>
                {line}
              </Text>
            </View>
          ))}
          {brief.length > 0 && text.summary.trim() ? (
            <Collapsible text={text.summary} lines={3} tone="secondary" variant="callout" />
          ) : null}
        </Card>
        <View style={{ paddingHorizontal: spacing.xs, gap: 2 }}>
          {asked ? (
            <Text variant="footnote" tone="secondary">
              Asked {relative(asked)}
            </Text>
          ) : null}
          {checked && decision.checked && decision.checked !== decision.asked ? (
            <Text variant="footnote" tone="secondary">
              Emberhut re-checked it {relative(checked)}
            </Text>
          ) : null}
          {stale ? (
            <Text variant="footnote" tone="warning">
              Not re-checked in over 12 hours, so it may be outdated. Emberhut re-checks it on its own.
            </Text>
          ) : null}
        </View>
      </View>
      {text.draft ? <Draft label={text.draftLabel ?? "Message to send"} draft={text.draft} /> : null}
      {text.after ? (
        <Card>
          <Collapsible text={text.after} lines={3} />
        </Card>
      ) : null}
      {cards ? (
        <View style={{ gap: spacing.md }}>
          <SectionHeading>Your options</SectionHeading>
          {choices.map((choice, i) =>
            choice.title ? (
              <ChoiceCard
                key={choice.label}
                choice={choice}
                pick={decision.recommend === i}
                media={(choice.media ?? []).flatMap((id) => artifacts.filter((a) => a.id === id))}
                onAnswer={() => onAnswer(choice.label)}
              />
            ) : null,
          )}
          <OptionRows
            decision={decision}
            options={choices.flatMap((c, i) => (c.title ? [] : [{ label: c.label, index: i }]))}
            onAnswer={onAnswer}
          />
        </View>
      ) : decision.options.length > 0 ? (
        <OptionRows
          decision={decision}
          options={decision.options.map((label, index) => ({ label, index }))}
          onAnswer={onAnswer}
          title="Your options"
        />
      ) : null}
    </>
  );
}

/** The message itself, as you'd send it: its own block, copied exactly. */
function Draft({ label, draft }: { label: string; draft: string }) {
  const { spacing } = useTheme();
  const [copied, setCopied] = useState(false);
  return (
    <View style={{ gap: spacing.sm }}>
      <View style={{ flexDirection: "row", alignItems: "center", paddingRight: spacing.xs }}>
        <SectionHeading style={{ flex: 1 }}>{label}</SectionHeading>
        <Pressable
          onPress={() =>
            inBackground(
              Clipboard.setStringAsync(draft).then(() => setCopied(true)),
              "Decision: setStringAsync",
            )
          }
          accessibilityRole="button"
          accessibilityLabel="Copy the message"
          hitSlop={10}
        >
          <Text variant="footnote" weight="600" tone="accent">
            {copied ? "Copied" : "Copy"}
          </Text>
        </Pressable>
      </View>
      <Card>
        <Collapsible text={draft} lines={5} />
      </Card>
    </View>
  );
}

/** One answer per row: what it is, what it does, Emberhut's pick marked. A tap answers. */
function OptionRows({
  decision,
  options,
  onAnswer,
  title,
}: {
  decision: Decision;
  options: { label: string; index: number }[];
  onAnswer: (choice: string) => void;
  title?: string;
}) {
  const { colors, spacing } = useTheme();
  return (
    <Section title={title}>
      {options.map(({ label, index }) => {
        const pick = decision.recommend === index;
        const note = decision.notes?.[index]?.trim();
        const shown = answerLabel(label);
        return (
          <Pressable
            key={label}
            onPress={() => onAnswer(label)}
            accessibilityRole="button"
            accessibilityLabel={pick ? `${shown}, Emberhut's pick` : shown}
            style={({ pressed }) => [
              styles.option,
              {
                gap: spacing.md,
                padding: spacing.md,
                paddingHorizontal: spacing.lg,
                backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
              },
            ]}
          >
            <View style={{ flex: 1, gap: 2 }}>
              <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm, flexWrap: "wrap" }}>
                <Text weight="600">{shown}</Text>
                {pick ? <Chip label="Emberhut's pick" tone="needsYou" strong /> : null}
              </View>
              {note ? (
                <Text variant="footnote" tone="secondary">
                  {note}
                </Text>
              ) : null}
            </View>
            <Icon name="arrow-up-circle" size={26} color={pick ? colors.needsYou : colors.textTertiary} />
          </Pressable>
        );
      })}
    </Section>
  );
}

/** One option: what you need to weigh it, and its button. */
function ChoiceCard({
  choice,
  pick,
  media,
  onAnswer,
}: {
  choice: DecisionChoice;
  pick: boolean;
  media: TaskArtifact[];
  onAnswer: () => void;
}) {
  const { colors, radius, spacing } = useTheme();
  const [more, setMore] = useState(false);
  const fields = choice.fields ?? [];
  const shown = more ? fields : fields.slice(0, 4);
  const long = fields.length > 4 || !!choice.more?.trim() || fields.some((f) => f.v.length > 140) || (choice.summary ?? "").length > 220;
  const title = choice.title ?? choice.label;
  return (
    <View
      accessibilityLabel={pick ? `${title}, Emberhut's pick` : title}
      style={{
        backgroundColor: colors.surface,
        borderRadius: radius.lg,
        padding: spacing.lg,
        gap: spacing.sm,
        borderWidth: pick ? 1.5 : 1,
        borderColor: pick ? colors.needsYou : colors.border,
      }}
    >
      <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm }}>
        <Text variant="headline" style={{ flex: 1 }}>
          {title}
        </Text>
        {choice.badge?.trim() ? <Chip label={choice.badge} tone={pick ? "needsYou" : "muted"} /> : null}
      </View>
      {pick ? (
        <View style={{ flexDirection: "row", gap: spacing.sm, alignItems: "flex-start" }}>
          <Chip label="Emberhut's pick" tone="needsYou" strong />
          {choice.why?.trim() ? (
            <Text variant="footnote" tone="needsYou" style={{ flex: 1 }}>
              {choice.why}
            </Text>
          ) : null}
        </View>
      ) : null}
      {choice.summary?.trim() ? (
        <Text variant="callout" numberOfLines={more ? undefined : 4}>
          {choice.summary}
        </Text>
      ) : null}
      {shown.map((field) => (
        <View key={field.k} style={{ gap: 2 }}>
          <Text variant="caption" weight="600" tone="secondary" style={{ textTransform: "uppercase" }}>
            {field.k}
          </Text>
          <Text variant="callout" numberOfLines={more ? undefined : 3}>
            {field.v}
          </Text>
        </View>
      ))}
      {more && choice.more?.trim() ? (
        <Text variant="callout" tone="secondary">
          {choice.more}
        </Text>
      ) : null}
      {media.length > 0 ? (
        <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ gap: spacing.sm }}>
          {media.map((a) => (
            <ServerImage
              key={a.id}
              path={a.thumb ?? a.url}
              style={{ width: 120, height: 80, borderRadius: radius.sm }}
              accessibilityLabel={a.caption ?? a.name}
            />
          ))}
        </ScrollView>
      ) : null}
      {long ? (
        <Pressable
          onPress={() => setMore(!more)}
          accessibilityRole="button"
          accessibilityLabel={`${more ? "Less" : "More"} about ${title}`}
          hitSlop={8}
        >
          <Text variant="footnote" weight="600" tone="accent">
            {more ? "Less" : "More"}
          </Text>
        </Pressable>
      ) : null}
      {choice.then?.trim() ? (
        <View style={{ flexDirection: "row", gap: spacing.xs }}>
          <Icon name="return-down-forward" size={14} color={colors.textSecondary} />
          <Text variant="footnote" tone="secondary" style={{ flex: 1 }}>
            {choice.then}
          </Text>
        </View>
      ) : null}
      <Button label={answerLabel(choice.label)} variant={pick ? "primary" : "secondary"} onPress={onAnswer} block />
    </View>
  );
}

const styles = StyleSheet.create({
  option: { minHeight: minTap, flexDirection: "row", alignItems: "center" },
});
