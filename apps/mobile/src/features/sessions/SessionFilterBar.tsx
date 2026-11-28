import { useState } from "react";
import { Modal, Pressable, ScrollView, StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { minTap, useTheme } from "@/theme";
import { Button, Icon, Text } from "@/ui";
import { chipLabel, toggle, type Pick, type SessionFilter, type SessionPicks } from "./sessionFilters";

type Menu = "worker" | "environment";

/**
 * The Sessions tab's Worker and Environment chips: plain with their name when nothing is picked, filled with what's
 * picked. A tap opens its sheet: All (clears it), then each pick with a tick and its count; it stays open for several picks.
 */
export function SessionFilterBar({
  filter,
  picks,
  onChange,
}: {
  filter: SessionFilter;
  picks: SessionPicks;
  onChange: (filter: SessionFilter) => void;
}) {
  const { spacing } = useTheme();
  const [open, setOpen] = useState<Menu | null>(null);
  const pickedWorkers = picks.workers.filter((p) => filter.workers?.includes(p.key));
  const pickedEnv = [
    ...picks.machines.filter((p) => filter.machines?.includes(p.key)),
    ...picks.accounts.filter((p) => filter.accounts?.includes(p.key)),
  ];
  return (
    <View style={[styles.bar, { gap: spacing.sm, paddingHorizontal: spacing.lg, paddingTop: spacing.md }]}>
      <FilterChip
        label={chipLabel("Worker", pickedWorkers)}
        on={pickedWorkers.length > 0}
        onPress={() => setOpen("worker")}
        name="Worker"
      />
      <FilterChip
        label={chipLabel("Environment", pickedEnv)}
        on={pickedEnv.length > 0}
        onPress={() => setOpen("environment")}
        name="Environment"
      />
      <PickSheet
        visible={open === "worker"}
        title="Worker"
        groups={[{ title: null, picks: picks.workers, picked: filter.workers, set: (workers) => onChange({ ...filter, workers }) }]}
        onAll={() => onChange({ ...filter, workers: null })}
        onClose={() => setOpen(null)}
      />
      <PickSheet
        visible={open === "environment"}
        title="Environment"
        groups={[
          { title: "Hut", picks: picks.machines, picked: filter.machines, set: (machines) => onChange({ ...filter, machines }) },
          { title: "Account", picks: picks.accounts, picked: filter.accounts, set: (accounts) => onChange({ ...filter, accounts }) },
        ]}
        onAll={() => onChange({ ...filter, machines: null, accounts: null })}
        onClose={() => setOpen(null)}
      />
    </View>
  );
}

function FilterChip({ label, on, onPress, name }: { label: string; on: boolean; onPress: () => void; name: string }) {
  const { colors, radius, spacing } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={on ? `${name}: ${label}` : `Filter by ${name.toLowerCase()}`}
      hitSlop={6}
      style={({ pressed }) => ({
        flexDirection: "row",
        alignItems: "center",
        gap: 4,
        paddingHorizontal: spacing.md,
        paddingVertical: 6,
        borderRadius: radius.pill,
        backgroundColor: on ? colors.accent : colors.surfaceRaised,
        opacity: pressed ? 0.7 : 1,
      })}
    >
      <Text variant="callout" weight={on ? "600" : "400"} numberOfLines={1} style={{ color: on ? colors.onAccent : colors.text }}>
        {label}
      </Text>
      <Icon name="chevron-down" size={14} color={on ? colors.onAccent : colors.textSecondary} />
    </Pressable>
  );
}

interface Group {
  title: string | null;
  picks: Pick[];
  picked: string[] | null | undefined;
  set: (list: string[] | null) => void;
}

function PickSheet({
  visible,
  title,
  groups,
  onAll,
  onClose,
}: {
  visible: boolean;
  title: string;
  groups: Group[];
  onAll: () => void;
  onClose: () => void;
}) {
  const { colors, radius, spacing } = useTheme();
  const insets = useSafeAreaInsets();
  const none = groups.every((g) => !g.picked || g.picked.length === 0);
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose} statusBarTranslucent navigationBarTranslucent>
      <Pressable style={styles.backdrop} onPress={onClose} accessibilityLabel="Close" accessibilityRole="button" />
      <View
        style={{
          maxHeight: "75%",
          backgroundColor: colors.surface,
          borderTopLeftRadius: radius.lg,
          borderTopRightRadius: radius.lg,
          paddingTop: spacing.sm,
          paddingBottom: insets.bottom + spacing.sm,
        }}
      >
        <Text variant="footnote" tone="secondary" weight="600" style={{ paddingHorizontal: spacing.lg, paddingVertical: spacing.sm }}>
          {title}
        </Text>
        <ScrollView>
          <PickRow label="All" checked={none} onPress={onAll} />
          {groups.map((g) => (
            <View key={g.title ?? "picks"}>
              {g.title && g.picks.length > 0 ? (
                <Text variant="caption" tone="tertiary" weight="600" style={{ paddingHorizontal: spacing.lg, paddingTop: spacing.md }}>
                  {g.title}
                </Text>
              ) : null}
              {g.picks.map((p) => (
                <PickRow
                  key={p.key}
                  label={p.label}
                  count={p.count}
                  checked={!!g.picked?.includes(p.key)}
                  onPress={() => g.set(toggle(g.picked, p.key))}
                />
              ))}
            </View>
          ))}
        </ScrollView>
        <View style={{ paddingHorizontal: spacing.lg, paddingTop: spacing.sm }}>
          <Button label="Done" onPress={onClose} />
        </View>
      </View>
    </Modal>
  );
}

function PickRow({ label, count, checked, onPress }: { label: string; count?: number; checked: boolean; onPress: () => void }) {
  const { colors, spacing } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="checkbox"
      accessibilityState={{ checked }}
      accessibilityLabel={count === undefined ? label : `${label}, ${count}`}
      style={({ pressed }) => [
        styles.row,
        { gap: spacing.md, paddingHorizontal: spacing.lg, backgroundColor: pressed ? colors.surfaceRaised : "transparent" },
      ]}
    >
      <Icon name={checked ? "checkmark-circle" : "ellipse-outline"} size={22} color={checked ? colors.accent : colors.textSecondary} />
      <Text style={styles.fill} numberOfLines={1}>
        {label}
      </Text>
      {count !== undefined && (
        <Text variant="callout" tone="secondary" style={{ fontVariant: ["tabular-nums"] }}>
          {count}
        </Text>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  bar: { flexDirection: "row", flexWrap: "wrap", alignItems: "center" },
  backdrop: { flex: 1, backgroundColor: "rgba(0,0,0,0.35)" },
  row: { minHeight: minTap + 4, flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
});
