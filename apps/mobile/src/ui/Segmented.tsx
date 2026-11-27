import { Pressable, View } from "react-native";
import { minTap, useTheme } from "@/theme";
import { Text } from "./Text";

export interface SegmentedProps<T extends string> {
  options: readonly { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
  accessibilityLabel: string;
  /** Dimmed and not pressable, while the choice doesn't apply. */
  disabled?: boolean;
}

/** One choice of a few, side by side. */
export function Segmented<T extends string>({ options, value, onChange, accessibilityLabel, disabled = false }: SegmentedProps<T>) {
  const { colors, radius, spacing } = useTheme();
  return (
    <View
      accessibilityRole="radiogroup"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ disabled }}
      style={{
        flexDirection: "row",
        backgroundColor: colors.surfaceRaised,
        borderRadius: radius.md,
        padding: 3,
        gap: 3,
        opacity: disabled ? 0.4 : 1,
      }}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <Pressable
            key={option.value}
            onPress={() => onChange(option.value)}
            disabled={disabled}
            accessibilityRole="radio"
            accessibilityState={{ selected, disabled }}
            accessibilityLabel={option.label}
            style={{
              flex: 1,
              minHeight: minTap - 6,
              alignItems: "center",
              justifyContent: "center",
              borderRadius: radius.sm,
              paddingHorizontal: spacing.sm,
              backgroundColor: selected ? colors.surface : "transparent",
            }}
          >
            <Text variant="callout" weight={selected ? "600" : "400"} tone={selected ? "primary" : "secondary"}>
              {option.label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}
