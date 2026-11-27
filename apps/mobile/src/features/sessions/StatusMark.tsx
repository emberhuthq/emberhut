import { useEffect, useState } from "react";
import { AccessibilityInfo, Animated, View } from "react-native";
import { inBackground } from "@/log";
import { useTheme } from "@/theme";
import { Icon } from "@/ui";
import { isRunning, sessionStatusText } from "./sessionList";

/** A session's state as tasks mark theirs: done a green tick, waiting on you your hand, stopped orange, failed red; running a pulsing dot. */
export function StatusMark({ status, size = 14 }: { status: string; size?: number }) {
  const { colors } = useTheme();
  if (isRunning(status)) return <LiveDot color={colors.done} />;
  const [name, color] =
    status === "waiting"
      ? (["hand-left", colors.needsYou] as const)
      : status === "failed"
        ? (["close-circle", colors.danger] as const)
        : status === "interrupted"
          ? (["stop-circle", colors.warning] as const)
          : (["checkmark-circle", colors.done] as const);
  return (
    <View accessible accessibilityLabel={sessionStatusText(status)}>
      <Icon name={name} size={size} color={color} />
    </View>
  );
}

export function LiveDot({ color }: { color: string }) {
  const [pulse] = useState(() => new Animated.Value(1));
  useEffect(() => {
    let loop: Animated.CompositeAnimation | null = null;
    inBackground(
      AccessibilityInfo.isReduceMotionEnabled().then((still) => {
        if (still) return;
        loop = Animated.loop(
          Animated.sequence([
            Animated.timing(pulse, { toValue: 0.3, duration: 800, useNativeDriver: true }),
            Animated.timing(pulse, { toValue: 1, duration: 800, useNativeDriver: true }),
          ]),
        );
        loop.start();
      }),
      "StatusMark: reduce motion",
    );
    return () => loop?.stop();
  }, [pulse]);
  return (
    <Animated.View
      accessible
      accessibilityLabel="Running"
      style={{ width: 8, height: 8, margin: 3, borderRadius: 4, backgroundColor: color, opacity: pulse }}
    />
  );
}
