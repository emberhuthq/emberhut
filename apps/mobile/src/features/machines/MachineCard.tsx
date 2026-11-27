import { useNavigation } from "@react-navigation/native";
import { Pressable, StyleSheet, View } from "react-native";
import type { Machine, MachineThresholds } from "@/api";
import { useTheme, type Palette } from "@/theme";
import { Card, Chip, Icon, ProgressBar, Text, type TextTone } from "@/ui";
import { meters, sessionRow, skipLine, stateTone, warningLevel, type Level, type Meter } from "./machines";

const levelColor: Record<Level, keyof Palette> = { ok: "working", warn: "warning", low: "danger" };
const levelTone: Record<Level, TextTone> = { ok: "secondary", warn: "warning", low: "danger" };

/** One machine: its name and state, why it takes no new work, its warnings, its bars and the sessions on it. */
export function MachineCard({ machine, limits }: { machine: Machine; limits: MachineThresholds }) {
  const { colors, spacing } = useTheme();
  const skip = skipLine(machine);
  const bars = meters(machine, limits);
  return (
    <Card accent={machine.state === "low" ? colors.danger : machine.state === "warn" ? colors.warning : undefined}>
      <View style={[styles.header, { gap: spacing.sm }]}>
        <View style={styles.grow}>
          <Text variant="headline">{machine.name}</Text>
          {machine.host ? (
            <Text variant="footnote" tone="secondary">
              {machine.host}
            </Text>
          ) : null}
        </View>
        <Chip label={machine.status} tone={stateTone(machine.state)} accessibilityLabel={`${machine.state}: ${machine.status}`} />
      </View>
      {skip ? (
        <Text variant="footnote" tone="danger" weight="600">
          {skip}
        </Text>
      ) : null}
      {machine.warnings.map((w, i) => (
        <Text key={i} variant="footnote" tone={levelTone[warningLevel(w)]} weight={w.level === "critical" ? "700" : "500"}>
          {w.text}
        </Text>
      ))}
      {bars.map((m) => (
        <MeterView key={m.key} meter={m} />
      ))}
      {machine.sessions.length > 0 ? <Sessions machine={machine} /> : null}
    </Card>
  );
}

function MeterView({ meter }: { meter: Meter }) {
  const { colors, spacing } = useTheme();
  return (
    <View style={{ gap: spacing.xs, paddingTop: spacing.xs }}>
      <View style={[styles.header, { gap: spacing.sm }]}>
        <Text variant="callout" weight="500" style={styles.grow} numberOfLines={1}>
          {meter.label}
        </Text>
        <Text variant="callout" tone={meter.level === "ok" ? "primary" : levelTone[meter.level]}>
          {meter.value}
        </Text>
      </View>
      <ProgressBar value={meter.share} color={colors[levelColor[meter.level]]} label={`${meter.label}: ${meter.value}`} />
      {meter.detail ? (
        <Text variant="caption" tone="secondary">
          {meter.detail}
        </Text>
      ) : null}
      {meter.lines?.map((line, i) => (
        <Text key={i} variant="caption" tone="secondary">
          {line}
        </Text>
      ))}
    </View>
  );
}

function Sessions({ machine }: { machine: Machine }) {
  const navigation = useNavigation();
  const { colors, spacing } = useTheme();
  return (
    <View style={{ gap: spacing.xs, paddingTop: spacing.sm }}>
      <Text variant="footnote" weight="600" tone="secondary">
        {machine.sessions.length === 1 ? "1 session" : `${machine.sessions.length} sessions`}
      </Text>
      {machine.sessions.map((s) => {
        const row = sessionRow(s);
        return (
          <Pressable
            key={s.id}
            accessibilityRole="button"
            onPress={() => navigation.navigate("Session", { id: s.id, title: row.title })}
            style={({ pressed }) => [styles.header, { gap: spacing.sm, paddingVertical: spacing.xs, opacity: pressed ? 0.6 : 1 }]}
          >
            <View style={styles.grow}>
              <Text variant="callout" numberOfLines={1}>
                {row.title}
              </Text>
              <Text variant="caption" tone="secondary" numberOfLines={2}>
                {row.subtitle}
              </Text>
            </View>
            <Icon name="chevron-forward" size={16} color={colors.textTertiary} />
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  header: { flexDirection: "row", alignItems: "center" },
  grow: { flex: 1 },
});
