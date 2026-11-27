import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, View } from "react-native";
import type { PartyPersona } from "@/api";
import { isOlderServer } from "@/features/machines/machines";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { api } from "@/services";
import { useParties } from "@/state/queries";
import { queryKeys } from "@/state/query";
import { useTheme } from "@/theme";
import { Button, EmptyState, ErrorState, Icon, KeyboardAvoider, ListRow, Loading, Screen, Section, Segmented, Text, TextField } from "@/ui";
import {
  addPersona,
  canStart,
  defaultModel,
  estimate,
  freeId,
  maxCast,
  modelName,
  notInCast,
  olderServerText,
  partyCost,
  partyTokens,
  savePersona,
  startingCast,
  stepTurns,
  tokenCaps,
  turnCaps,
} from "./party";
import { Initial } from "./PartyParts";
import { AddPersonaSheet, PersonaEditor } from "./PersonaSheets";

/** A new party: the topic, the cast (ready-made, your own from before, or new), each one's model, and the caps. */
export function NewPartyScreen({ navigation }: RootScreenProps<"NewParty">) {
  const { colors, spacing } = useTheme();
  const client = useQueryClient();
  const page = useParties();
  const [topic, setTopic] = useState("");
  // Until you change it, the cast is the optimist, the skeptic and the engineer.
  const [picked, setPicked] = useState<PartyPersona[] | null>(null);
  const [turns, setTurns] = useState<number | null>(null);
  const [tokens, setTokens] = useState<number | null>(null);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<PartyPersona | null>(null);
  const [starting, setStarting] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  if (page.isPending) return <Loading label="Looking…" />;
  if (!page.data) {
    if (isOlderServer(page.error)) return <EmptyState icon="people-outline" title="Not yet" message={olderServerText} />;
    return (
      <ErrorState
        message={page.error?.message ?? "Couldn't read the personas"}
        onRetry={() => inBackground(page.refetch(), "NewPartyScreen: refetch")}
      />
    );
  }
  const { presets, recent, models, defaults } = page.data;
  const cast = picked ?? startingCast(presets);
  const maxTurns = turns ?? defaults.maxTurns;
  const maxTokens = tokens ?? defaults.maxTokens;
  const caps = tokenCaps.map((n) => ({ value: String(n), label: partyTokens(n) }));

  const start = async () => {
    setStarting(true);
    setProblem(null);
    try {
      const reply = await api.parties.start({
        topic: topic.trim(),
        personas: cast.map((p) => ({ name: p.name, role: p.role, model: p.model })),
        maxTurns,
        maxTokens,
      });
      inBackground(client.invalidateQueries({ queryKey: queryKeys.parties }), "NewPartyScreen: invalidateQueries");
      navigation.replace("Party", { id: reply.party.id, title: reply.party.title });
    } catch (cause) {
      setProblem(cause instanceof Error ? cause.message : "Couldn't start it");
      setStarting(false);
    }
  };

  return (
    <Screen padded={false} edges={["bottom"]}>
      <KeyboardAvoider>
        <ScrollView contentContainerStyle={{ padding: spacing.lg, gap: spacing.lg }} keyboardShouldPersistTaps="handled">
          <Section title="Topic or opening line">
            <TextField
              key="topic"
              value={topic}
              onChangeText={setTopic}
              placeholder="What should they talk about?"
              accessibilityLabel="Topic"
              multiline
              style={{ minHeight: 66 }}
            />
          </Section>
          <Section
            title={`Cast · ${cast.length} of 2 to 5`}
            footer="Tap one to change their role or model. Opus 5.5 talks best; Sonnet and Haiku cost less. All on AWS Bedrock."
          >
            {cast.map((p) => (
              <Pressable
                key={p.id}
                onPress={() => setEditing(p)}
                accessibilityRole="button"
                accessibilityLabel={`${p.name}, ${modelName(models, p.model)}. Change them`}
                style={({ pressed }) => [
                  styles.row,
                  {
                    gap: spacing.md,
                    padding: spacing.md,
                    paddingLeft: spacing.lg,
                    backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
                  },
                ]}
              >
                <Initial name={p.name} color={colors.accent} size={26} />
                <View style={styles.fill}>
                  <View style={[styles.row, { gap: spacing.sm }]}>
                    <Text weight="500" numberOfLines={1} style={styles.fill}>
                      {p.name}
                    </Text>
                    <Text variant="caption" tone="secondary">
                      {modelName(models, p.model)}
                    </Text>
                  </View>
                  <Text variant="caption" tone="secondary" numberOfLines={2}>
                    {p.role}
                  </Text>
                </View>
                <Pressable
                  onPress={() => setPicked(cast.filter((c) => c.id !== p.id))}
                  accessibilityRole="button"
                  accessibilityLabel={`Remove ${p.name}`}
                  hitSlop={10}
                >
                  <Icon name="remove-circle-outline" size={22} color={colors.danger} />
                </Pressable>
              </Pressable>
            ))}
            {cast.length < maxCast && (
              <ListRow key="add" icon="add-circle-outline" title="Add a persona" opens={false} onPress={() => setAdding(true)} />
            )}
          </Section>
          <Section title="Limits" footer={`At most about ${partyCost(estimate(cast, maxTokens, models))} at these caps`}>
            <View key="turns" style={[styles.row, { gap: spacing.md, paddingHorizontal: spacing.lg, paddingVertical: spacing.sm }]}>
              <Text style={styles.fill}>{`Up to ${maxTurns} turns`}</Text>
              <StepButton
                icon="remove"
                label="Fewer turns"
                disabled={maxTurns <= turnCaps.min}
                onPress={() => setTurns(stepTurns(maxTurns, -turnCaps.step))}
              />
              <StepButton
                icon="add"
                label="More turns"
                disabled={maxTurns >= turnCaps.max}
                onPress={() => setTurns(stepTurns(maxTurns, turnCaps.step))}
              />
            </View>
            <View key="tokens" style={{ gap: spacing.sm, padding: spacing.md, paddingHorizontal: spacing.lg }}>
              <Text>Token cap</Text>
              <Segmented accessibilityLabel="Token cap" options={caps} value={String(maxTokens)} onChange={(v) => setTokens(Number(v))} />
            </View>
          </Section>
          {problem && <Text tone="danger">{problem}</Text>}
          <Button
            label="Start"
            icon="play"
            block
            loading={starting}
            disabled={!canStart(topic, cast)}
            onPress={() => inBackground(start(), "NewPartyScreen: start")}
          />
        </ScrollView>
      </KeyboardAvoider>
      {adding && (
        <AddPersonaSheet
          presets={notInCast(presets, cast)}
          recent={notInCast(recent, cast)}
          onPick={(p) => {
            setPicked(addPersona(cast, p));
            setAdding(false);
          }}
          onNew={() => {
            setAdding(false);
            setEditing({ id: freeId(cast, "new"), name: "", role: "", model: defaults.model || defaultModel });
          }}
          onClose={() => setAdding(false)}
        />
      )}
      {editing && (
        <PersonaEditor persona={editing} models={models} onSave={(p) => setPicked(savePersona(cast, p))} onClose={() => setEditing(null)} />
      )}
    </Screen>
  );
}

function StepButton({ icon, label, disabled, onPress }: { icon: "add" | "remove"; label: string; disabled: boolean; onPress: () => void }) {
  const { colors, radius } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={({ pressed }) => [
        styles.step,
        { borderRadius: radius.sm, backgroundColor: colors.surfaceRaised },
        (pressed || disabled) && { opacity: disabled ? 0.4 : 0.7 },
      ]}
    >
      <Icon name={icon} size={20} color={colors.text} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1, gap: 2 },
  step: { width: 44, height: 36, alignItems: "center", justifyContent: "center" },
});
