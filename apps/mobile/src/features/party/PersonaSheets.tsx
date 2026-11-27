import { useState, type ReactNode } from "react";
import { Modal, Pressable, ScrollView, StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { PartyModel, PartyPersona } from "@/api";
import { minTap, useTheme } from "@/theme";
import { Icon, KeyboardAvoider, ListRow, Section, Text, TextField } from "@/ui";
import { shortRole } from "./party";

/** A page over the new party: a bar with Cancel (and Done), then what scrolls. */
function Sheet({
  title,
  onClose,
  done,
  children,
}: {
  title: string;
  onClose: () => void;
  done?: { onPress: () => void; disabled: boolean };
  children: ReactNode;
}) {
  const { colors, spacing } = useTheme();
  const insets = useSafeAreaInsets();
  return (
    <Modal visible animationType="slide" onRequestClose={onClose} statusBarTranslucent navigationBarTranslucent>
      <View style={[styles.fill, { backgroundColor: colors.background, paddingTop: insets.top, paddingBottom: insets.bottom }]}>
        <KeyboardAvoider>
          <View style={[styles.bar, { paddingHorizontal: spacing.lg, backgroundColor: colors.surface, gap: spacing.md }]}>
            <BarButton label="Cancel" onPress={onClose} />
            <Text variant="headline" numberOfLines={1} align="center" style={styles.fill}>
              {title}
            </Text>
            {done ? <BarButton label="Done" onPress={done.onPress} disabled={done.disabled} strong /> : <View style={{ width: 56 }} />}
          </View>
          <ScrollView contentContainerStyle={{ padding: spacing.lg, gap: spacing.lg }} keyboardShouldPersistTaps="handled">
            {children}
          </ScrollView>
        </KeyboardAvoider>
      </View>
    </Modal>
  );
}

function BarButton({
  label,
  onPress,
  disabled = false,
  strong = false,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  strong?: boolean;
}) {
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      hitSlop={8}
      style={[styles.barButton, { alignItems: strong ? "flex-end" : "flex-start" }, disabled && { opacity: 0.4 }]}
    >
      <Text tone="accent" weight={strong ? "600" : "400"}>
        {label}
      </Text>
    </Pressable>
  );
}

/** Who to add: the ready-made ones, your own from past parties, or someone new. */
export function AddPersonaSheet({
  presets,
  recent,
  onPick,
  onNew,
  onClose,
}: {
  presets: PartyPersona[];
  recent: PartyPersona[];
  onPick: (p: PartyPersona) => void;
  onNew: () => void;
  onClose: () => void;
}) {
  const row = (p: PartyPersona) => (
    <ListRow key={p.id} title={p.name} subtitle={shortRole(p.role)} icon="person-outline" opens={false} onPress={() => onPick(p)} />
  );
  return (
    <Sheet title="Add a persona" onClose={onClose}>
      <Section>
        <ListRow key="new" title="New persona…" icon="create-outline" opens={false} onPress={onNew} />
      </Section>
      <Section title="Ready-made">{presets.map(row)}</Section>
      <Section title="Yours">{recent.map(row)}</Section>
    </Sheet>
  );
}

/** One persona: a name, who they are and how they talk (their system prompt), and their model. */
export function PersonaEditor({
  persona,
  models,
  onSave,
  onClose,
}: {
  persona: PartyPersona;
  models: PartyModel[];
  onSave: (p: PartyPersona) => void;
  onClose: () => void;
}) {
  const { colors } = useTheme();
  const [name, setName] = useState(persona.name);
  const [role, setRole] = useState(persona.role);
  const [model, setModel] = useState(persona.model);
  const save = () => {
    onSave({ ...persona, name, role, model });
    onClose();
  };
  return (
    <Sheet title={persona.name || "New persona"} onClose={onClose} done={{ onPress: save, disabled: name.trim() === "" }}>
      <Section title="Name">
        <TextField key="name" value={name} onChangeText={setName} placeholder="Name" accessibilityLabel="Name" />
      </Section>
      <Section title="Role" footer={'e.g. "A blunt startup investor who has seen a thousand pitches."'}>
        <TextField
          key="role"
          value={role}
          onChangeText={setRole}
          placeholder="Who they are and how they talk"
          accessibilityLabel="Role"
          multiline
          style={{ minHeight: 88 }}
        />
      </Section>
      <Section title="Model">
        {models.map((m) => (
          <ListRow
            key={m.id}
            title={m.name}
            subtitle={m.note}
            opens={false}
            onPress={() => setModel(m.id)}
            trailing={m.id === model ? <Icon name="checkmark" size={20} color={colors.accent} accessibilityLabel="Chosen" /> : undefined}
          />
        ))}
      </Section>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  bar: { minHeight: minTap + 8, flexDirection: "row", alignItems: "center" },
  barButton: { minWidth: 56, minHeight: minTap, justifyContent: "center" },
});
