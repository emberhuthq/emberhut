import type { ReactNode } from "react";
import { ActivityIndicator, StyleSheet, View } from "react-native";
import { useTheme } from "@/theme";
import { Button } from "./Button";
import { Icon, type IconName } from "./Icon";
import { Text } from "./Text";

export interface EmptyStateProps {
  title: string;
  message?: string;
  icon?: IconName;
  action?: ReactNode;
}

/** Nothing to show here, said plainly, with what you can do about it. */
export function EmptyState({ title, message, icon, action }: EmptyStateProps) {
  const { colors, spacing } = useTheme();
  return (
    <View style={[styles.center, { gap: spacing.md, padding: spacing.xl }]}>
      {icon && <Icon name={icon} size={40} color={colors.textTertiary} />}
      <Text variant="headline" align="center">
        {title}
      </Text>
      {message ? (
        <Text tone="secondary" align="center">
          {message}
        </Text>
      ) : null}
      {action}
    </View>
  );
}

export function ErrorState({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return (
    <EmptyState
      icon="cloud-offline-outline"
      title="Something went wrong"
      message={message}
      action={onRetry ? <Button label="Try again" variant="secondary" onPress={onRetry} /> : undefined}
    />
  );
}

export function Loading({ label }: { label?: string }) {
  const { colors, spacing } = useTheme();
  return (
    <View style={[styles.center, { gap: spacing.md }]} accessibilityRole="progressbar" accessibilityLabel={label ?? "Loading"}>
      <ActivityIndicator color={colors.accent} size="large" />
      {label ? <Text tone="secondary">{label}</Text> : null}
    </View>
  );
}

const styles = StyleSheet.create({ center: { flex: 1, alignItems: "center", justifyContent: "center" } });
