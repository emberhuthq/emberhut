import { useEffect, useId } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useStore } from "zustand";
import { createStore } from "zustand/vanilla";
import { actionsStore } from "@/services";
import { useActions } from "@/state/hooks";
import { useTheme } from "@/theme";
import { Icon, Text } from "@/ui";

/** Above the tab bar. */
const tabBarClearance = 64;

/** Bars pinned at the bottom of a page (a task's answers), by who pinned them: the toast sits above the tallest. */
const pinned = createStore<{ bars: ReadonlyMap<string, number> }>(() => ({ bars: new Map() }));

/** For a bar pinned at a page's bottom: keeps the toast above it; returns the bar's onLayout. */
export function useLiftToast(): (height: number) => void {
  const id = useId();
  useEffect(
    () => () =>
      pinned.setState((s) => {
        const bars = new Map(s.bars);
        bars.delete(id);
        return { bars };
      }),
    [id],
  );
  return (height) => {
    if (pinned.getState().bars.get(id) !== height) pinned.setState((s) => ({ bars: new Map(s.bars).set(id, height) }));
  };
}

/**
 * What a tap on a task did, with Undo for the few seconds before it is sent; a refusal says why and brings the task back.
 * One for the whole app, over every screen.
 */
export function UndoToast() {
  const { colors, radius, spacing } = useTheme();
  const insets = useSafeAreaInsets();
  const toast = useActions((s) => s.toast);
  const bar = useStore(pinned, (s) => Math.max(0, ...s.bars.values()));
  if (!toast) return null;
  // A pinned bar already has the bottom inset in its height.
  const bottom = bar > 0 ? bar + spacing.sm : insets.bottom + tabBarClearance;
  const tone = toast.failed ? colors.danger : toast.warn ? colors.warning : colors.done;
  return (
    <View
      pointerEvents="box-none"
      style={[StyleSheet.absoluteFill, { justifyContent: "flex-end", padding: spacing.md, paddingBottom: bottom }]}
    >
      <View
        accessibilityLiveRegion="polite"
        style={[
          styles.toast,
          {
            gap: spacing.md,
            padding: spacing.md,
            paddingLeft: spacing.lg,
            borderRadius: radius.lg,
            backgroundColor: colors.surfaceRaised,
            borderColor: colors.border,
          },
        ]}
      >
        <Icon name={toast.failed ? "alert-circle" : toast.warn ? "close-circle" : "checkmark-circle"} size={20} color={tone} />
        <Text variant="callout" style={styles.text} numberOfLines={3}>
          {toast.text}
        </Text>
        {toast.sending ? <ActivityIndicator color={colors.accent} /> : null}
        {toast.failed ? (
          <Pressable onPress={() => actionsStore.getState().dismiss()} hitSlop={10} accessibilityRole="button" accessibilityLabel="Dismiss">
            <Icon name="close" size={20} color={colors.textSecondary} />
          </Pressable>
        ) : !toast.sending ? (
          <Pressable
            onPress={() => actionsStore.getState().undo()}
            hitSlop={10}
            accessibilityRole="button"
            accessibilityLabel={`Undo: ${toast.text}`}
          >
            <Text weight="700" tone="accent">
              Undo
            </Text>
          </Pressable>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  toast: {
    flexDirection: "row",
    alignItems: "center",
    borderWidth: StyleSheet.hairlineWidth,
    elevation: 6,
  },
  text: { flex: 1 },
});
