import { Modal, Pressable, StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { minTap, useTheme } from "@/theme";
import { Icon, type IconName } from "./Icon";
import { Text } from "./Text";

export interface SheetAction {
  label: string;
  icon?: IconName;
  destructive?: boolean;
  onPress: () => void;
}

export interface ActionSheetProps {
  visible: boolean;
  title?: string;
  actions: SheetAction[];
  onClose: () => void;
}

/** What a long press offers, from the bottom of the screen; a tap outside closes it. */
export function ActionSheet({ visible, title, actions, onClose }: ActionSheetProps) {
  const { colors, radius, spacing } = useTheme();
  const insets = useSafeAreaInsets();
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose} statusBarTranslucent navigationBarTranslucent>
      <Pressable style={styles.backdrop} onPress={onClose} accessibilityLabel="Close" accessibilityRole="button" />
      <View
        style={{
          backgroundColor: colors.surface,
          borderTopLeftRadius: radius.lg,
          borderTopRightRadius: radius.lg,
          paddingTop: spacing.sm,
          paddingBottom: insets.bottom + spacing.sm,
        }}
      >
        {title ? (
          <Text
            variant="footnote"
            tone="secondary"
            numberOfLines={2}
            style={{ paddingHorizontal: spacing.lg, paddingVertical: spacing.sm }}
          >
            {title}
          </Text>
        ) : null}
        {actions.map((action) => (
          <Pressable
            key={action.label}
            onPress={() => {
              onClose();
              action.onPress();
            }}
            accessibilityRole="button"
            style={({ pressed }) => [
              styles.row,
              { gap: spacing.md, paddingHorizontal: spacing.lg, backgroundColor: pressed ? colors.surfaceRaised : "transparent" },
            ]}
          >
            {action.icon && <Icon name={action.icon} size={20} color={action.destructive ? colors.danger : colors.textSecondary} />}
            <Text tone={action.destructive ? "danger" : "primary"}>{action.label}</Text>
          </Pressable>
        ))}
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: { flex: 1, backgroundColor: "rgba(0,0,0,0.35)" },
  row: { minHeight: minTap + 4, flexDirection: "row", alignItems: "center" },
});
