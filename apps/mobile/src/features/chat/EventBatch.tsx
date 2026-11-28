import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { useTheme } from "@/theme";
import { Icon, Text } from "@/ui";
import { eventLines, eventSummary, noticedLine } from "./rows";

/**
 * What the server told the ember (events, check-ins): not conversation, so one quiet centered line, `Juniper noticed · …`
 * (`noticer`: the ember's name, or the room's agent); a tap lists them.
 */
export function EventBatch({ text, noticer }: { text: string; noticer: string }) {
  const { colors, radius, spacing } = useTheme();
  const [open, setOpen] = useState(false);
  const lines = eventLines(text);
  const summary = lines.length > 0 ? eventSummary(lines) : text;
  return (
    <View style={[styles.center, { gap: spacing.sm }]}>
      <Pressable
        onPress={() => setOpen((o) => !o)}
        disabled={lines.length === 0}
        accessibilityRole="button"
        accessibilityLabel={`${noticer} noticed: ${summary}`}
        accessibilityHint={open ? "Hides the list" : `Shows what ${noticer} noticed`}
        hitSlop={8}
        style={[styles.row, { gap: spacing.xs }]}
      >
        <Text variant="caption" tone="secondary" numberOfLines={1}>
          {noticedLine(noticer, summary)}
        </Text>
        {lines.length > 0 && <Icon name={open ? "chevron-down" : "chevron-forward"} size={12} color={colors.textSecondary} />}
      </Pressable>
      {open && (
        <View
          style={[styles.list, { gap: spacing.sm, padding: spacing.md, borderRadius: radius.md, backgroundColor: colors.surfaceRaised }]}
        >
          {lines.map((line, i) => (
            <View key={i} style={[styles.row, { gap: spacing.sm }]}>
              <Icon
                name={line.kind === "checkin" ? "alarm-outline" : "ellipse"}
                size={line.kind === "checkin" ? 14 : 6}
                color={colors.textTertiary}
              />
              <Text variant="footnote" tone="secondary" numberOfLines={2} style={styles.shrink}>
                {line.text}
              </Text>
            </View>
          ))}
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  center: { alignItems: "center" },
  row: { flexDirection: "row", alignItems: "center" },
  list: { alignSelf: "stretch", maxWidth: 440 },
  shrink: { flexShrink: 1 },
});
