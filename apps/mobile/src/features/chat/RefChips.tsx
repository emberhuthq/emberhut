import * as Clipboard from "expo-clipboard";
import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Linking, Pressable, StyleSheet, View } from "react-native";
import type { MessageRef, TaskStatus } from "@/api";
import { inBackground } from "@/log";
import { useTheme, type Palette } from "@/theme";
import { ActionSheet, Icon, Text, type IconName, type SheetAction } from "@/ui";
import { refTarget, refText, shownRefs } from "./refs";

const icons: Record<MessageRef["kind"], IconName> = {
  task: "checkbox-outline",
  pr: "git-pull-request-outline",
  ticket: "ticket-outline",
  card: "albums-outline",
  report: "newspaper-outline",
  file: "document-outline",
};

/** What one of Emberhut's answers is about: small chips under it. A task's opens its page, a report the viewer, a PR its link. */
export function RefChips({ refs, taskStatus }: { refs: readonly MessageRef[]; taskStatus: (id: number) => TaskStatus | undefined }) {
  const { colors, radius, spacing } = useTheme();
  const [expanded, setExpanded] = useState(false);
  const { shown, more } = shownRefs(refs, expanded);
  return (
    <View style={[styles.wrap, { gap: spacing.xs + 2 }]}>
      {shown.map((ref, i) => (
        <RefChip key={`${ref.kind}-${ref.label}-${i}`} item={ref} status={ref.task !== undefined ? taskStatus(ref.task) : undefined} />
      ))}
      {more > 0 && (
        <Pressable
          onPress={() => setExpanded(true)}
          accessibilityRole="button"
          accessibilityLabel={`${more} more`}
          hitSlop={6}
          style={({ pressed }) => [
            styles.chip,
            {
              paddingHorizontal: spacing.sm + 1,
              paddingVertical: spacing.xs + 1,
              borderRadius: radius.pill,
              backgroundColor: colors.surfaceRaised,
              opacity: pressed ? 0.7 : 1,
            },
          ]}
        >
          <Text variant="footnote" weight="600" tone="secondary">
            +{more}
          </Text>
        </Pressable>
      )}
    </View>
  );
}

// A task's chip says where it stands: done green, waiting on you in your color, being worked blue.
function tint(colors: Palette, status: TaskStatus | undefined): string {
  switch (status) {
    case "done":
      return colors.done;
    case "waiting":
      return colors.needsYou;
    case "working":
      return colors.working;
    default:
      return colors.textSecondary;
  }
}

function RefChip({ item: ref, status }: { item: MessageRef; status: TaskStatus | undefined }) {
  const { colors, radius, spacing } = useTheme();
  const navigation = useNavigation();
  const [menu, setMenu] = useState(false);
  const target = refTarget(ref);
  const open = () => {
    if (target.open === "task") navigation.navigate("Task", { id: target.id });
    else if (target.open === "file") navigation.navigate("File", { path: target.path, name: target.name, kind: "file" });
    else if (target.open === "link") inBackground(Linking.openURL(target.url), "RefChips: openURL");
  };
  const actions: SheetAction[] = [
    ...(target.open === "link"
      ? [
          {
            label: "Copy link",
            icon: "link-outline" as const,
            onPress: () => inBackground(Clipboard.setStringAsync(target.url), "RefChips: setStringAsync"),
          },
        ]
      : []),
    ...(ref.kind === "task"
      ? [
          {
            label: `Copy ${ref.label}`,
            icon: "copy-outline" as const,
            onPress: () => inBackground(Clipboard.setStringAsync(ref.label), "RefChips: setStringAsync"),
          },
        ]
      : []),
  ];
  const text = refText(ref);
  return (
    <>
      <Pressable
        onPress={open}
        onLongPress={actions.length > 0 ? () => setMenu(true) : undefined}
        disabled={target.open === "none"}
        accessibilityRole={target.open === "link" ? "link" : "button"}
        accessibilityLabel={text}
        style={({ pressed }) => [
          styles.chip,
          {
            gap: spacing.xs,
            paddingHorizontal: spacing.sm + 1,
            paddingVertical: spacing.xs + 1,
            borderRadius: radius.pill,
            backgroundColor: colors.surfaceRaised,
            opacity: pressed ? 0.7 : 1,
          },
        ]}
      >
        <Icon name={icons[ref.kind]} size={14} color={tint(colors, status)} />
        <Text variant="footnote" numberOfLines={1} style={styles.shrink}>
          {text}
        </Text>
      </Pressable>
      <ActionSheet visible={menu} title={text} actions={actions} onClose={() => setMenu(false)} />
    </>
  );
}

const styles = StyleSheet.create({
  wrap: { flexDirection: "row", flexWrap: "wrap" },
  chip: { flexDirection: "row", alignItems: "center", maxWidth: "100%" },
  shrink: { flexShrink: 1 },
});
