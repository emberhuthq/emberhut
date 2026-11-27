import { Pressable, StyleSheet, View } from "react-native";
import type { SessionStep } from "@/api";
import { useTheme } from "@/theme";
import { Icon, Markdown, Text } from "@/ui";

/** What a session asks you (AskUserQuestion): each question with its options; while it waits, a tap answers. */
export function QuestionCard({ step, answer }: { step: SessionStep; answer: ((text: string) => void) | null }) {
  const { colors, radius, spacing } = useTheme();
  const questions = step.questions ?? [];
  return (
    <View style={{ gap: spacing.md, padding: spacing.md, borderRadius: radius.md, borderWidth: 1, borderColor: colors.needsYou }}>
      <View style={[styles.row, { gap: spacing.xs }]}>
        <Icon name="help-circle-outline" size={16} color={colors.needsYou} />
        <Text variant="caption" weight="600" tone="needsYou">
          {answer ? "It asks you" : "It asked you"}
        </Text>
      </View>
      {questions.map((q, i) => (
        <View key={i} style={{ gap: spacing.xs + 2 }}>
          {q.header ? (
            <Text variant="caption" tone="secondary" weight="600">
              {q.header.toUpperCase()}
            </Text>
          ) : null}
          <Markdown text={q.question} variant="callout" />
          {q.options.map((o) => (
            <Pressable
              key={o.label}
              onPress={answer ? () => answer(questions.length > 1 ? `${q.question} ${o.label}` : o.label) : undefined}
              disabled={!answer}
              accessibilityRole="button"
              accessibilityLabel={o.description ? `${o.label}: ${o.description}` : o.label}
              style={({ pressed }) => [
                {
                  gap: 2,
                  paddingHorizontal: spacing.md,
                  paddingVertical: spacing.sm,
                  borderRadius: radius.sm,
                  backgroundColor: colors.surfaceRaised,
                  opacity: !answer ? 0.6 : pressed ? 0.7 : 1,
                },
              ]}
            >
              <Text variant="callout" weight="600">
                {o.label}
              </Text>
              {o.description ? (
                <Text variant="footnote" tone="secondary">
                  {o.description}
                </Text>
              ) : null}
            </Pressable>
          ))}
        </View>
      ))}
      {answer && (
        <Text variant="caption" tone="secondary">
          Or write your own answer below.
        </Text>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
});
