import { useState } from "react";
import { Pressable, StyleSheet, TextInput, View } from "react-native";
import { useTheme } from "@/theme";
import { Icon, Text, type IconName } from "@/ui";
import { keyScript, textScript, typedChange } from "./scripts";

/** While you drive: what you type goes straight into the page as you type it, with the keys a phone keyboard lacks. */
export function DriveKeys({ run }: { run: (script: string) => void }) {
  const { colors, radius, spacing, type } = useTheme();
  const [text, setText] = useState("");
  const change = (next: string) => {
    const { backspaces, text: added } = typedChange(text, next);
    for (let i = 0; i < backspaces; i++) run(keyScript("Backspace", "Backspace", 8));
    if (added) run(textScript(added));
    setText(next);
  };
  const keys: { label: string; icon?: IconName; key: [string, string, number] }[] = [
    { label: "Tab", key: ["Tab", "Tab", 9] },
    { label: "Enter", key: ["Enter", "Enter", 13] },
    { label: "Esc", key: ["Escape", "Escape", 27] },
    { label: "Left", icon: "arrow-back", key: ["ArrowLeft", "ArrowLeft", 37] },
    { label: "Up", icon: "arrow-up", key: ["ArrowUp", "ArrowUp", 38] },
    { label: "Down", icon: "arrow-down", key: ["ArrowDown", "ArrowDown", 40] },
    { label: "Right", icon: "arrow-forward", key: ["ArrowRight", "ArrowRight", 39] },
  ];
  return (
    <View style={{ gap: spacing.sm }}>
      <TextInput
        value={text}
        onChangeText={change}
        autoFocus
        autoCapitalize="none"
        autoCorrect={false}
        onSubmitEditing={() => {
          run(keyScript("Enter", "Enter", 13));
          setText("");
        }}
        submitBehavior="submit"
        placeholder="Type into the page"
        placeholderTextColor={colors.textTertiary}
        accessibilityLabel="Type into the page"
        style={[
          type.callout,
          {
            minHeight: 40,
            paddingHorizontal: spacing.md,
            borderRadius: radius.sm,
            color: colors.text,
            backgroundColor: colors.surfaceRaised,
          },
        ]}
      />
      <View style={[styles.row, { gap: spacing.xs }]}>
        {keys.map((k) => (
          <Pressable
            key={k.label}
            onPress={() => run(keyScript(...k.key))}
            accessibilityRole="button"
            accessibilityLabel={k.label}
            style={({ pressed }) => [
              styles.key,
              { borderRadius: radius.sm, backgroundColor: pressed ? colors.border : colors.surfaceRaised },
            ]}
          >
            {k.icon ? <Icon name={k.icon} size={16} color={colors.text} /> : <Text variant="footnote">{k.label}</Text>}
          </Pressable>
        ))}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", flexWrap: "wrap" },
  key: { minWidth: 44, height: 36, paddingHorizontal: 10, alignItems: "center", justifyContent: "center" },
});
