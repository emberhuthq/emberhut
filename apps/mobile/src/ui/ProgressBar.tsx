import { View } from "react-native";
import { useTheme } from "@/theme";

/** How far along, 0…1, as a thin bar. */
export function ProgressBar({ value, color, label }: { value: number; color?: string; label?: string }) {
  const { colors, radius } = useTheme();
  const shown = Math.min(Math.max(value, 0), 1);
  return (
    <View
      accessibilityRole="progressbar"
      accessibilityLabel={label}
      accessibilityValue={{ min: 0, max: 100, now: Math.round(shown * 100) }}
      style={{ height: 4, borderRadius: radius.pill, backgroundColor: colors.surfaceRaised, overflow: "hidden" }}
    >
      <View style={{ width: `${shown * 100}%`, height: "100%", backgroundColor: color ?? colors.accent }} />
    </View>
  );
}
