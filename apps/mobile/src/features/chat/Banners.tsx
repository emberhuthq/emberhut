import { Pressable, StyleSheet, View } from "react-native";
import { hostOf } from "@/features/settings/servers";
import { inBackground } from "@/log";
import { fallbackStore } from "@/services";
import { useFallback, useSession } from "@/state/hooks";
import { useTheme } from "@/theme";
import { Button, Icon, Text, type IconName } from "@/ui";

function Banner({
  icon,
  color,
  title,
  detail,
  action,
}: {
  icon: IconName;
  color: string;
  title: string;
  detail?: string;
  action?: React.ReactNode;
}) {
  const { colors, radius, spacing } = useTheme();
  return (
    <View
      style={[
        styles.row,
        { gap: spacing.md, padding: spacing.md, borderRadius: radius.lg, backgroundColor: colors.surface, borderColor: colors.border },
      ]}
    >
      <Icon name={icon} color={color} />
      <View style={styles.fill}>
        <Text variant="callout" weight="600">
          {title}
        </Text>
        {detail ? (
          <Text variant="footnote" tone="secondary">
            {detail}
          </Text>
        ) : null}
      </View>
      {action}
    </View>
  );
}

/** Paused: said where you type, with the one button that undoes it. */
export function PausedBanner({ onResume, resuming }: { onResume: () => void; resuming: boolean }) {
  const { colors } = useTheme();
  return (
    <Banner
      icon="pause-circle"
      color={colors.textSecondary}
      title="Emberhut is paused"
      detail="It wakes for nothing until you resume it. It still answers you here."
      action={<Button label="Resume" onPress={onResume} loading={resuming} />}
    />
  );
}

export function OfflineBanner({ reason }: { reason: string }) {
  const { colors } = useTheme();
  return (
    <Banner
      icon="cloud-offline-outline"
      color={colors.warning}
      title={reason}
      detail="Showing what Emberhut said last. It tries again every second."
    />
  );
}

/**
 * On the fallback server: the server stopped answering, so requests go to the fallback server until Try again finds the
 * server answering (the app never goes back by itself).
 */
export function FallbackBanner() {
  const { colors } = useTheme();
  const server = useSession((s) => s.serverUrl);
  const fallback = useSession((s) => s.fallback?.url);
  const checking = useFallback((s) => s.checking);
  const stillDown = useFallback((s) => s.stillDown);
  const tryAgain = () => inBackground(fallbackStore.getState().tryAgain(), "FallbackBanner: tryAgain");
  return (
    <Banner
      icon="swap-horizontal"
      color={colors.warning}
      title={`${server ? hostOf(server) : "Your server"} isn't answering`}
      detail={`You're on the fallback server${fallback ? `, ${hostOf(fallback)}` : ""}.${stillDown ? " Still no answer: try again later." : ""}`}
      action={<Button label="Try again" onPress={tryAgain} loading={checking} />}
    />
  );
}

/** Emberhut handed you the browser: one tap opens it, where you do the step and hands it back. */
export function HandoffBanner({ line, onOpen }: { line: string; onOpen: () => void }) {
  const { colors, radius, spacing } = useTheme();
  return (
    <Pressable
      onPress={onOpen}
      accessibilityRole="button"
      accessibilityLabel={`${line}. Opens the browser`}
      style={({ pressed }) => [
        styles.row,
        {
          gap: spacing.md,
          padding: spacing.md,
          borderRadius: radius.lg,
          backgroundColor: colors.surface,
          borderColor: colors.needsYou,
          opacity: pressed ? 0.8 : 1,
        },
      ]}
    >
      <Icon name="hand-left" color={colors.needsYou} />
      <Text variant="callout" weight="600" numberOfLines={2} style={styles.fill}>
        {line}
      </Text>
      <Icon name="chevron-forward" size={16} color={colors.textTertiary} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", borderWidth: StyleSheet.hairlineWidth },
  fill: { flex: 1 },
});
