import { useEffect, useState } from "react";
import { AccessibilityInfo, Animated, Easing, StyleSheet, View } from "react-native";
import { inBackground } from "@/log";
import { useTheme } from "@/theme";
import { Face, Text } from "@/ui";

/** Emberhut is working: its face, gently breathing, and what it's doing now. */
export function Working({ doing }: { doing: string | null }) {
  const { spacing } = useTheme();
  const [breath] = useState(() => new Animated.Value(0));
  useEffect(() => {
    let loop: Animated.CompositeAnimation | null = null;
    inBackground(
      AccessibilityInfo.isReduceMotionEnabled().then((still) => {
        if (still) return;
        loop = Animated.loop(
          Animated.sequence([
            Animated.timing(breath, { toValue: 1, duration: 1_400, easing: Easing.inOut(Easing.sin), useNativeDriver: true }),
            Animated.timing(breath, { toValue: 0, duration: 1_400, easing: Easing.inOut(Easing.sin), useNativeDriver: true }),
          ]),
        );
        loop.start();
      }),
      "Working: reduce motion",
    );
    return () => loop?.stop();
  }, [breath]);
  const label = doing || "Emberhut is thinking";
  return (
    <View
      style={[styles.row, { gap: spacing.sm + 2, paddingVertical: spacing.xs }]}
      accessible
      accessibilityLiveRegion="polite"
      accessibilityLabel={`Emberhut is working: ${doing || "thinking"}`}
    >
      <Animated.View style={{ transform: [{ scale: breath.interpolate({ inputRange: [0, 1], outputRange: [1, 1.06] }) }] }}>
        <Face size={26} />
      </Animated.View>
      <Animated.View style={[styles.shrink, { opacity: breath.interpolate({ inputRange: [0, 1], outputRange: [0.6, 1] }) }]}>
        <Text variant="callout" tone="secondary" numberOfLines={1}>
          {label}
        </Text>
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  shrink: { flexShrink: 1 },
});
