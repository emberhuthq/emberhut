import { Pressable, View } from "react-native";
import { minTap, useTheme } from "@/theme";
import { Text } from "./Text";

export interface SegmentedProps<T extends string> {
  options: readonly { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
  accessibilityLabel: string;
}

/** One choice of a few, side by side. */
export function Segmented<T extends string>({ options, value, onChange, accessibilityLabel }: SegmentedProps<T>) {
  const { colors, radius, spacing } = useTheme();
  return (
    <View
      accessibilityRole="radiogroup"
      accessibilityLabel={accessibilityLabel}
      style={{ flexDirection: "row", backgroundColor: colors.surfaceRaised, borderRadius: radius.md, padding: 3, gap: 3 }}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <Pressable
            key={option.value}
            onPress={() => onChange(option.value)}
            accessibilityRole="radio"
            accessibilityState={{ selected }}
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
