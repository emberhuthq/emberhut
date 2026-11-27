import { Switch } from "react-native";
import { useTheme } from "@/theme";

/** An on/off switch in the app's colors. */
export function Toggle({
  value,
  onChange,
  label,
  disabled = false,
}: {
  value: boolean;
  onChange: (on: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <Switch
      value={value}
      onValueChange={onChange}
      disabled={disabled}
      accessibilityLabel={label}
      trackColor={{ false: colors.border, true: colors.primary }}
      thumbColor={value ? colors.switchThumb : colors.textSecondary}
    />
  );
}
