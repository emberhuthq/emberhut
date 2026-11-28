import * as Haptics from "expo-haptics";
import { Pressable, StyleSheet, View } from "react-native";
import type { AgentRow } from "@/api";
import { agentCaption } from "@/features/approvals/model";
import { inBackground } from "@/log";
import { useTheme } from "@/theme";
import { Icon, Text } from "@/ui";
import type { EmberhutStatus } from "./status";

/**
 * The header: the room ("Emberhut" for Main) and what Emberhut is doing there; a tap lists the rooms. An agent's room is marked
 * as one, with who the agent works for beside its status.
 */
export function ChatTitle({
  name,
  status,
  agent,
  onPress,
}: {
  name: string;
  status: EmberhutStatus;
  agent?: AgentRow;
  onPress: () => void;
}) {
  const { colors, spacing } = useTheme();
  const caption = agent ? agentCaption(agent, status.text) : status.text;
  return (
    <Pressable
      onPress={() => {
        inBackground(Haptics.selectionAsync(), "ChatTitle: selectionAsync");
        onPress();
      }}
      accessibilityRole="button"
      accessibilityLabel={agent ? `${name}, agent, ${caption}` : `${name}, ${status.text}`}
      accessibilityHint="Switches rooms"
      hitSlop={8}
      style={({ pressed }) => [styles.title, pressed && styles.pressed]}
    >
      <View style={[styles.row, { gap: spacing.xs }]}>
        {agent && <Icon name="person-circle-outline" size={16} color={agent.enabled ? colors.accent : colors.textTertiary} />}
        <Text variant="headline" numberOfLines={1} maxFontSizeMultiplier={barTextScale} style={styles.shrink}>
          {name}
        </Text>
        <Icon name="chevron-down" size={14} color={colors.textSecondary} />
      </View>
      <Text
        variant="caption"
        tone={agent && !agent.enabled ? "tertiary" : status.tone}
        numberOfLines={1}
        maxFontSizeMultiplier={barTextScale}
      >
        {caption}
      </Text>
    </Pressable>
  );
}

/**
 * How far the title block grows with Dynamic Type: it lives in the 44 pt navigation bar, as UIKit's own titles do, and
 * those keep their size. The headline's 22 pt line and the caption's 16 pt one fill 44 pt at 1.15×; any larger and the
 * block spills out of the bar, off the centre the bar's buttons line up with.
 */
const barTextScale = 1.15;

const styles = StyleSheet.create({
  title: { maxWidth: 220, alignItems: "center" },
  pressed: { opacity: 0.5 },
  row: { flexDirection: "row", alignItems: "center" },
  shrink: { flexShrink: 1 },
});
