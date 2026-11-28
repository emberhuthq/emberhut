import { useEffect, useState } from "react";
import { AccessibilityInfo, Animated, Easing, StyleSheet, View } from "react-native";
import type { Issue } from "@/api";
import { inBackground } from "@/log";
import { mix } from "@/theme/colors";
import { useTheme } from "@/theme";
import { Text } from "@/ui";
import { isRunningTask } from "./plan";
import { isBlocked, type Segment } from "./planView";

/** Whether the system asks for less motion; animations stay still then. */
function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(true);
  useEffect(() => {
    inBackground(AccessibilityInfo.isReduceMotionEnabled().then(setReduced), "PlanMarks: reduce motion");
    const listener = AccessibilityInfo.addEventListener("reduceMotionChanged", setReduced);
    return () => listener.remove();
  }, []);
  return reduced;
}

/**
 * Running, as a shape and not colour alone: a ring in the running colour, a quarter of it solid, turning once every
 * 900 ms (still when the system asks for less motion).
 */
export function Spinner({ size = 14, color }: { size?: number; color?: string }) {
  const { colors } = useTheme();
  const tint = color ?? colors.running;
  const reduced = useReducedMotion();
  const [turn] = useState(() => new Animated.Value(0));
  useEffect(() => {
    if (reduced) return;
    const loop = Animated.loop(Animated.timing(turn, { toValue: 1, duration: 900, easing: Easing.linear, useNativeDriver: true }));
    loop.start();
    return () => loop.stop();
  }, [reduced, turn]);
  const rotate = turn.interpolate({ inputRange: [0, 1], outputRange: ["0deg", "360deg"] });
  return (
    <Animated.View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        borderWidth: Math.max(2, size / 7),
        borderColor: mix(tint, colors.surface, 0.3),
        borderTopColor: tint,
        borderRightColor: tint,
        transform: [{ rotate }],
      }}
    />
  );
}

/** A filled dot: what waits on you, in the theme's accent. */
function Dot({ color, size = 10 }: { color: string; size?: number }) {
  return <View style={{ width: size, height: size, borderRadius: size / 2, backgroundColor: color }} />;
}

/** A ring: open, not started (`dashed` for a step left out). */
export function Ring({ color, size = 14, dashed = false }: { color: string; size?: number; dashed?: boolean }) {
  return (
    <View
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        borderWidth: 1.6,
        borderColor: color,
        borderStyle: dashed ? "dashed" : "solid",
      }}
    />
  );
}

/** Stuck: a filled disc in the danger colour with a "!" on it. */
export function BlockedMark({ size = 15 }: { size?: number }) {
  const { colors } = useTheme();
  return (
    <View style={[styles.center, { width: size, height: size, borderRadius: size / 2, backgroundColor: colors.danger }]}>
      <Text weight="700" style={{ color: colors.surface, fontSize: size * 0.68, lineHeight: size }}>
        !
      </Text>
    </View>
  );
}

/** What a task's row leads with: needs you a filled ember dot, blocked the red mark, running the spinner, queued a still quarter ring, open a ring. */
export function TaskLead({ task }: { task: Issue }) {
  const { colors } = useTheme();
  const mark =
    task.status === "waiting" ? (
      <Dot color={colors.needsYou} />
    ) : isBlocked(task) ? (
      <BlockedMark />
    ) : task.live?.state === "running" ? (
      <Spinner />
    ) : isRunningTask(task) ? (
      <View style={[styles.quarter, { borderColor: mix(colors.info, colors.surface, 0.3), borderTopColor: colors.info }]} />
    ) : (
      <Ring color={colors.textTertiary} />
    );
  return (
    <View style={styles.lead} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      {mark}
    </View>
  );
}

/**
 * How far along, as a bar: a segment per step (done filled, the one being worked on in the running colour, a blocked one
 * red, the rest empty), or one plain bar past twelve steps. `label` is read out with its value.
 */
export function SegmentBar({
  segments,
  fraction,
  blocked = false,
  height = 4,
  label,
}: {
  segments: Segment[] | null;
  fraction: number;
  blocked?: boolean;
  height?: number;
  label: string;
}) {
  const { colors, radius } = useTheme();
  const fill: Record<Segment, string> = { done: colors.done, doing: colors.running, blocked: colors.danger, todo: colors.border };
  const shown = Math.min(Math.max(fraction, 0), 1);
  return (
    <View
      accessible
      accessibilityRole="progressbar"
      accessibilityLabel={label}
      accessibilityValue={{ min: 0, max: 100, now: Math.round(shown * 100) }}
      style={[styles.bar, { height, gap: 2 }]}
    >
      {segments ? (
        segments.map((s, i) => <View key={i} style={{ flex: 1, height, borderRadius: 1.5, backgroundColor: fill[s] }} />)
      ) : (
        <View style={{ flex: 1, height, borderRadius: radius.pill, backgroundColor: colors.border, overflow: "hidden" }}>
          <View style={{ width: `${shown * 100}%`, height, backgroundColor: blocked ? colors.danger : colors.done }} />
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  center: { alignItems: "center", justifyContent: "center" },
  lead: { width: 18, height: 22, alignItems: "center", justifyContent: "center" },
  quarter: { width: 14, height: 14, borderRadius: 7, borderWidth: 2 },
  bar: { flexDirection: "row", flex: 1, alignItems: "center" },
});
