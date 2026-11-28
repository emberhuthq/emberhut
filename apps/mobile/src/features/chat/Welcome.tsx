import { StyleSheet, View } from "react-native";
import { useEmberTint } from "@/features/ember/useEmberTint";
import { useTheme } from "@/theme";
import { Button, Face, Text } from "@/ui";

const starters = ["What should I look at today?", "What did you do while I was away?", "Check on the build in an hour"];

/** An empty chat: the face, one line, and three things to say so there is nothing to learn. */
export function Welcome({ onStarter }: { onStarter: (text: string) => void }) {
  const { spacing } = useTheme();
  const tint = useEmberTint();
  return (
    <View style={[styles.center, { gap: spacing.lg, paddingVertical: spacing.xxl }]}>
      <Face size={88} tint={tint} />
      <Text tone="secondary" align="center" style={styles.line}>
        Emberhut watches your work and speaks up when something matters.
      </Text>
      <View style={{ gap: spacing.sm }}>
        {starters.map((s) => (
          <Button key={s} label={s} variant="secondary" onPress={() => onStarter(s)} />
        ))}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  center: { alignItems: "center" },
  line: { maxWidth: 300 },
});
