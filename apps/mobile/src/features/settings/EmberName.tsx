import { useState } from "react";
import { StyleSheet, View } from "react-native";
import { customEmberName, DEFAULT_EMBER_NAME, EMBER_NAME_MAX, cleanEmberName } from "@/features/ember/name";
import { inBackground } from "@/log";
import { offlineReason } from "@/state/connection";
import { useConnection } from "@/state/hooks";
import { useEmberhutState, useRenameEmber } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, Section, Text, TextField } from "@/ui";

/**
 * Your ember's name: a field to rename it, Save (or Return) to keep it, and a way back to "Your ember". The new name shows
 * at once in the tab and the chat; one the server refuses goes back, with the reason under the field.
 */
export function EmberNameRow() {
  const { spacing } = useTheme();
  const offline = useConnection(offlineReason);
  const custom = customEmberName(useEmberhutState().data);
  const rename = useRenameEmber();
  const [draft, setDraft] = useState(custom ?? "");
  const [shown, setShown] = useState(custom);
  const [problem, setProblem] = useState<string | null>(null);
  // The name changed elsewhere (another device, or a refusal put it back): the field follows unless you typed over it.
  if (custom !== shown) {
    setShown(custom);
    if (draft === (shown ?? "")) setDraft(custom ?? "");
  }
  const clean = cleanEmberName(draft);
  const changed = !clean.ok || clean.name !== (custom ?? null);
  const save = (typed: string) => {
    setProblem(null);
    inBackground(
      rename.mutateAsync(typed).then((outcome) => {
        if (!outcome.ok) setProblem(`Not renamed: ${outcome.error}`);
      }),
      "EmberNameRow: rename",
    );
  };
  const reset = () => {
    setDraft("");
    save("");
  };
  return (
    <Section title="Name" footer={`What you call your ember, on every device. Up to ${EMBER_NAME_MAX} characters.`}>
      <View style={{ gap: spacing.sm, padding: spacing.md }}>
        <View style={[styles.row, { gap: spacing.sm }]}>
          <TextField
            value={draft}
            onChangeText={(text) => {
              setDraft(text);
              setProblem(null);
            }}
            onSubmitEditing={() => save(draft)}
            placeholder={DEFAULT_EMBER_NAME}
            autoCorrect={false}
            returnKeyType="done"
            maxLength={EMBER_NAME_MAX + 8}
            accessibilityLabel="Your ember's name"
            style={styles.fill}
          />
          <Button label="Save" onPress={() => save(draft)} disabled={!changed || offline !== null} loading={rename.isPending} />
        </View>
        {problem ? (
          <Text variant="footnote" tone="warning">
            {problem}
          </Text>
        ) : null}
        {custom !== undefined ? (
          <View style={styles.row}>
            <Button label={`Go back to “${DEFAULT_EMBER_NAME}”`} variant="plain" onPress={reset} disabled={offline !== null} />
          </View>
        ) : null}
      </View>
    </Section>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
});
