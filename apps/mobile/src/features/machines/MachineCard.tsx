import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { ClaudeAccount, Machine, MachineThresholds } from "@/api";
import { useTheme, type Palette } from "@/theme";
import { FoldRow, Icon, ProgressBar, Section, Text, type TextTone } from "@/ui";
import {
  asleepNote,
  asOf,
  hutKind,
  kindIcon,
  kindWord,
  pillLine,
  presenceOf,
  resourcesLine,
  room,
  sessionsLine,
  tinted,
  track,
  type Presence,
  type RoomTone,
} from "./huts";
import { meters, sessionRow, warningLevel, type Level, type Meter } from "./machines";

const levelColor: Record<Level, keyof Palette> = { ok: "working", warn: "warning", low: "danger" };
const levelTone: Record<Level, TextTone> = { ok: "secondary", warn: "warning", low: "danger" };
const presenceInk: Record<Presence, keyof Palette> = { online: "done", quiet: "warning", asleep: "textSecondary", notYet: "textSecondary" };
const roomColor: Record<RoomTone, keyof Palette> = { fine: "done", low: "warning", out: "danger", unknown: "textSecondary" };

/**
 * One hut as an iOS grouped section: its kind's icon, name and whether it is online or asleep (said once, in the pill);
 * its sessions and resources (an asleep hut's dimmed, with their age); a refusal or warning only for a hut that should be
 * there; its accounts with the room each has left; and folded under Details its bars and the sessions on it.
 */
export function MachineCard({ machine, limits, accounts }: { machine: Machine; limits: MachineThresholds; accounts: ClaudeAccount[] }) {
  const { spacing } = useTheme();
  const [open, setOpen] = useState(false);
  const presence = presenceOf(machine);
  const asleep = presence === "asleep";
  const resources = resourcesLine(machine);
  const age = asOf(machine);
  const there = presence === "online" || presence === "quiet";
  const skip = there && !machine.placeable ? `Not taking new sessions: ${machine.skipWhy ?? "it isn't reachable"}` : null;
  return (
    <Section>
      <Head key="head" machine={machine} />
      <Fact key="sessions" label="Sessions" value={sessionsLine(machine)} />
      {resources ? <Fact key="resources" label="Resources" value={age ? `${resources} · ${age}` : resources} dim={asleep} /> : null}
      {skip || (there && machine.warnings.length > 0) ? (
        <View key="warnings" style={{ paddingHorizontal: spacing.lg, paddingVertical: spacing.sm, gap: spacing.xs }}>
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
        </View>
      ) : null}
      {accounts.length === 0 ? (
        <View key="no-accounts" style={{ paddingHorizontal: spacing.lg, paddingVertical: spacing.md }}>
          <Text variant="callout" tone="secondary">
            No assistant accounts on this hut
          </Text>
        </View>
      ) : (
        accounts.map((a) => <AccountRow key={`account-${a.id}`} account={a} asleep={asleep} />)
      )}
      <FoldRow key="details" title="Details · bars and sessions" open={open} onToggle={() => setOpen(!open)} />
      {open ? <Details key="more" machine={machine} limits={limits} /> : null}
    </Section>
  );
}

/** The hut's head: its kind's icon on a neutral tile, its name, kind and host (and that an asleep one waits), and the pill. */
function Head({ machine }: { machine: Machine }) {
  const { colors, spacing, radius } = useTheme();
  const kind = hutKind(machine);
  const presence = presenceOf(machine);
  const pill = pillLine(machine);
  const { fill, text } = tinted(colors[presenceInk[presence]], colors.surface);
  const subtitle = machine.host && machine.host !== machine.name ? `${kindWord[kind]} · ${machine.host}` : kindWord[kind];
  return (
    <View
      accessible
      accessibilityLabel={`${machine.name}, ${kindWord[kind]}, ${pill}`}
      style={[styles.row, { gap: spacing.md, paddingHorizontal: spacing.lg, paddingVertical: spacing.md }]}
    >
      <View style={[styles.tile, { borderRadius: radius.sm, backgroundColor: colors.surfaceRaised }]}>
        <Icon name={kindIcon(kind, machine.battery !== null)} size={22} color={colors.textSecondary} />
      </View>
      <View style={styles.grow}>
        <Text variant="headline" numberOfLines={1}>
          {machine.name}
        </Text>
        <Text variant="footnote" tone="secondary" numberOfLines={1}>
          {subtitle}
        </Text>
        {presence === "asleep" ? (
          <Text variant="footnote" tone="secondary" numberOfLines={2}>
            {asleepNote}
          </Text>
        ) : null}
      </View>
      <View
        style={[
          styles.row,
          styles.pill,
          { gap: spacing.xs, paddingHorizontal: spacing.sm, borderRadius: radius.pill, backgroundColor: fill },
        ]}
      >
        <View style={[styles.dot, { backgroundColor: text }]} />
        <Text variant="caption" weight="600" style={{ color: text }} numberOfLines={1}>
          {pill}
        </Text>
      </View>
    </View>
  );
}

/** A label on the left and its value on the right, as Settings' rows; `dim` for figures that are old. */
function Fact({ label, value, dim = false }: { label: string; value: string; dim?: boolean }) {
  const { spacing } = useTheme();
  return (
    <View
      accessible
      accessibilityLabel={`${label}: ${value}`}
      style={[styles.row, { gap: spacing.md, paddingHorizontal: spacing.lg, paddingVertical: spacing.md }]}
    >
      <Text tone="secondary">{label}</Text>
      <Text style={[styles.grow, styles.right]} tone={dim ? "secondary" : "primary"} numberOfLines={2}>
        {value}
      </Text>
    </View>
  );
}

/** An account: its name (and address, when it has one), the room it has left in its colour with an outlined bar, and its
 * 5-hour and week room. */
function AccountRow({ account, asleep }: { account: ClaudeAccount; asleep: boolean }) {
  const { colors, spacing, radius } = useTheme();
  const r = room(account, new Date(), asleep);
  const color = colors[roomColor[r.tone]];
  const words = tinted(color, colors.surface).text;
  const bar = track(colors.textSecondary, colors.surface);
  return (
    <View
      accessible
      accessibilityLabel={[account.name, r.value, r.detail].filter(Boolean).join(", ")}
      style={{ paddingHorizontal: spacing.lg, paddingVertical: spacing.md, gap: spacing.xs }}
    >
      <View style={[styles.row, { gap: spacing.md }]}>
        <View style={styles.grow}>
          <Text numberOfLines={1}>{account.name}</Text>
          {account.email ? (
            <Text variant="caption" tone="secondary" numberOfLines={1}>
              {account.email}
            </Text>
          ) : null}
        </View>
        <Text weight="500" style={{ color: words }}>
          {r.value}
        </Text>
      </View>
      {r.share !== null ? (
        <View
          style={{
            height: 6,
            borderRadius: radius.pill,
            borderWidth: 1,
            borderColor: bar.line,
            backgroundColor: bar.fill,
            overflow: "hidden",
          }}
        >
          <View style={{ width: `${r.share * 100}%`, height: "100%", backgroundColor: color }} />
        </View>
      ) : null}
      {r.detail ? (
        <Text variant="caption" tone="secondary">
          {r.detail}
        </Text>
      ) : null}
    </View>
  );
}

/** Its bars and the sessions on it, under Details. */
function Details({ machine, limits }: { machine: Machine; limits: MachineThresholds }) {
  const { spacing } = useTheme();
  const bars = meters(machine, limits);
  return (
    <View style={{ paddingHorizontal: spacing.lg, paddingBottom: spacing.md, gap: spacing.xs }}>
      {bars.map((m) => (
        <MeterView key={m.key} meter={m} />
      ))}
      {machine.sessions.length > 0 ? <Sessions machine={machine} /> : null}
      {bars.length === 0 && machine.sessions.length === 0 ? (
        <Text variant="footnote" tone="secondary">
          Nothing measured yet
        </Text>
      ) : null}
    </View>
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
  row: { flexDirection: "row", alignItems: "center" },
  grow: { flex: 1 },
  right: { textAlign: "right" },
  tile: { width: 38, height: 38, alignItems: "center", justifyContent: "center" },
  pill: { paddingVertical: 2 },
  dot: { width: 6, height: 6, borderRadius: 3 },
});
