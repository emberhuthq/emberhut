import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { monoFont, useTheme } from "@/theme";
import { Markdown, Text } from "@/ui";
import { isLong } from "./transcript";

const shownLines = 12;

/** A tool's input or output, monospaced, a few lines until "More". */
export function CodeBlock({ title, text, error = false }: { title: string; text: string; error?: boolean }) {
  const { colors, radius, spacing } = useTheme();
  const [all, setAll] = useState(false);
  const long = text.split("\n").length > shownLines || text.length > 900;
  return (
    <View style={{ gap: 3, padding: spacing.sm, borderRadius: radius.sm, backgroundColor: colors.surfaceRaised }}>
      <Text variant="caption" tone="secondary" weight="600">
        {title}
      </Text>
      <Text
        variant="caption"
        selectable
        numberOfLines={all ? undefined : shownLines}
        style={[styles.mono, { color: error ? colors.danger : colors.text }]}
      >
        {text}
      </Text>
      {long && <More open={all} onPress={() => setAll((a) => !a)} />}
    </View>
  );
}

/** An edit's changed lines: removed in red, added in green, a few until "More". */
export function DiffBlock({ text, limit = 16 }: { text: string; limit?: number }) {
  const { colors, radius, spacing } = useTheme();
  const [all, setAll] = useState(false);
  const lines = text.split("\n");
  return (
    <View style={{ padding: spacing.sm, borderRadius: radius.sm, backgroundColor: colors.surfaceRaised }}>
      {(all ? lines : lines.slice(0, limit)).map((l, i) => {
        const removed = l.startsWith("- ");
        const added = l.startsWith("+ ");
        return (
          <Text
            key={i}
            variant="caption"
            numberOfLines={all ? undefined : 1}
            style={[styles.mono, { color: removed ? colors.danger : added ? colors.done : colors.textSecondary }]}
          >
            {l === "" ? " " : l}
          </Text>
        );
      })}
      {lines.length > limit && <More open={all} onPress={() => setAll((a) => !a)} label={`${lines.length - limit} more lines`} />}
    </View>
  );
}

function More({ open, onPress, label = "More" }: { open: boolean; onPress: () => void; label?: string }) {
  return (
    <Pressable onPress={onPress} accessibilityRole="button" accessibilityLabel={open ? "Show less" : label} hitSlop={8} style={styles.more}>
      <Text variant="caption" tone="accent" weight="600">
        {open ? "Less" : label}
      </Text>
    </Pressable>
  );
}

/** Claude's words: markdown, a long answer kept short until "Show more". */
export function LongText({ text, onLink }: { text: string; onLink: (url: string) => void }) {
  const [open, setOpen] = useState(false);
  const long = isLong(text);
  const shown = open || !long ? text : text.split("\n\n").slice(0, 3).join("\n\n");
  return (
    <View style={styles.gap}>
      <Markdown text={shown} taskRefs onLink={onLink} selectable />
      {long && (
        <Pressable
          onPress={() => setOpen((o) => !o)}
          accessibilityRole="button"
          accessibilityLabel={open ? "Show less" : "Show more"}
          style={styles.more}
        >
          <Text variant="callout" tone="accent" weight="600">
            {open ? "Show less" : "Show more"}
          </Text>
        </Pressable>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  mono: { fontFamily: monoFont },
  more: { alignSelf: "flex-start", paddingTop: 4 },
  gap: { gap: 6 },
});
