import { Pressable, View } from "react-native";
import { minTap, useTheme } from "@/theme";
import { Icon, type IconName } from "./Icon";
import { Text } from "./Text";

export interface SegmentedProps<T extends string> {
  /** Each choice; one with an icon shows the icon alone, its label read out. */
  options: readonly { value: T; label: string; icon?: IconName }[];
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
        backgroundColor: colors.track,
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
              borderWidth: 1,
              borderColor: selected ? colors.raisedEdge : "transparent",
              backgroundColor: selected ? colors.raised : "transparent",
            }}
          >
            {option.icon ? (
              <Icon name={option.icon} size={18} color={selected ? colors.text : colors.textSecondary} />
            ) : (
              <Text variant="callout" weight={selected ? "600" : "400"} tone={selected ? "primary" : "secondary"}>
                {option.label}
              </Text>
            )}
          </Pressable>
        );
      })}
    </View>
  );
}
