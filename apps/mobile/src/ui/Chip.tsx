import { View } from "react-native";
import { useTheme, type Palette } from "@/theme";
import { Icon, type IconName } from "./Icon";
import { Text } from "./Text";

/** `running`: a run going on a task with plans, in the plans' running colour beside their spinner. */
export type ChipTone = "needsYou" | "working" | "running" | "done" | "warning" | "danger" | "muted";

export interface ChipProps {
  label: string;
  tone?: ChipTone;
  /** Filled with its tone; otherwise a soft tint. */
  strong?: boolean;
  icon?: IconName;
  accessibilityLabel?: string;
}

const toneColor: Record<ChipTone, keyof Palette> = {
  needsYou: "needsYou",
  working: "working",
  running: "running",
  done: "done",
  warning: "warning",
  danger: "danger",
  muted: "textSecondary",
};

/** A small label in a capsule: a state, a deadline, Emberhut's pick. */
export function Chip({ label, tone = "muted", strong = false, icon, accessibilityLabel }: ChipProps) {
  const { colors, radius, spacing } = useTheme();
  const color = colors[toneColor[tone]];
  // Filled: the accent's own text colour on the accent, the status one on the others.
  const text = strong ? (tone === "needsYou" ? colors.onAccent : colors.onStatus) : color;
  return (
    <View
      accessibilityLabel={accessibilityLabel}
      style={{
        flexDirection: "row",
        alignItems: "center",
        alignSelf: "flex-start",
        gap: spacing.xs,
        paddingHorizontal: spacing.sm,
        paddingVertical: 2,
        borderRadius: radius.pill,
        backgroundColor: strong ? color : `${color}26`,
      }}
    >
      {icon && <Icon name={icon} size={12} color={text} />}
      <Text variant="caption" weight="600" style={{ color: text }} numberOfLines={1}>
        {label}
      </Text>
    </View>
  );
}
