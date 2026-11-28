import { useState, type ReactNode } from "react";
import { Pressable, ScrollView, StyleSheet, View } from "react-native";
import type { RootScreenProps } from "@/navigation/types";
import { useTheme } from "@/theme";
import { Icon, ListRow, Section, SectionHeading, Text, TextField } from "@/ui";
import { groupName, groupShown, groups, groupStatus, pathOf, type Destination, type Now } from "./entries";
import { search, suggestions, type Row } from "./find";
import { useSettingsNow } from "./useSettingsNow";

/**
 * Settings asks first: a search over every setting, the groups as chips (they scroll sideways, the edge fading) that open
 * their screens, and Suggested, a handful of plain verbs worked out from how things stand. Typing narrows the list to
 * what matches, each saying where it lives; a tap opens that place and changes nothing by itself.
 */
export function SettingsScreen({ navigation }: RootScreenProps<"Settings">) {
  const { colors, spacing } = useTheme();
  const now = useSettingsNow();
  const [query, setQuery] = useState("");
  const typed = query.trim() !== "";
  const found = typed ? search(query, now) : null;
  const rows = found ? found.rows : suggestions(now);
  const open = (to: Destination) => {
    if ("group" in to) navigation.navigate("SettingsGroup", to.focus ? { group: to.group, focus: to.focus } : { group: to.group });
    else if (to.screen === "Account") navigation.navigate("Account");
    else navigation.navigate(to.screen);
  };
  return (
    <View style={{ flex: 1, backgroundColor: colors.background }}>
      <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: spacing.lg, gap: spacing.lg }}>
        {now.hasAccount ? (
          <Section>
            <ListRow
              key="account"
              icon="person-circle-outline"
              title="Account"
              subtitle="Your emberhut.com account · optional"
              onPress={() => navigation.navigate("Account")}
            />
          </Section>
        ) : null}
        <View style={[styles.search, { gap: spacing.sm }]}>
          <TextField
            value={query}
            onChangeText={setQuery}
            placeholder="What do you want to change?"
            accessibilityLabel="Search settings"
            autoCorrect={false}
            autoCapitalize="none"
            clearButtonMode="while-editing"
            returnKeyType="search"
            onSubmitEditing={() => {
              const top = rows[0];
              if (top) open(top.entry.to);
            }}
          />
        </View>
        <Chips now={now} onOpen={(group) => open({ group })} />
        {found?.nothing ? <Text tone="secondary">Nothing called that. Try a group above.</Text> : null}
        {rows.length > 0 ? (
          <View style={{ gap: spacing.sm }}>
            {typed ? null : <SectionHeading>Suggested</SectionHeading>}
            <Section>
              {rows.map((row) => (
                <ResultRow key={row.entry.id} row={row} onPress={() => open(row.entry.to)} />
              ))}
            </Section>
          </View>
        ) : null}
      </ScrollView>
    </View>
  );
}

/** The groups' chips, in a row that scrolls sideways, its right edge fading into the page. */
function Chips({ now, onOpen }: { now: Now; onOpen: (group: (typeof groups)[number]) => void }) {
  const { colors, radius, spacing } = useTheme();
  // The account has its row at the top.
  const shown = groups.filter((group) => group !== "account" && groupShown(group, now));
  return (
    <View>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ gap: spacing.sm, paddingRight: 32 }}>
        {shown.map((group) => {
          const status = groupStatus(group, now);
          return (
            <Pressable
              key={group}
              accessibilityRole="button"
              accessibilityLabel={status ? `${groupName[group]}, ${status}` : groupName[group]}
              onPress={() => onOpen(group)}
              style={({ pressed }) => ({
                flexDirection: "row",
                alignItems: "center",
                gap: spacing.xs,
                paddingHorizontal: spacing.md,
                paddingVertical: spacing.xs + 2,
                borderRadius: radius.pill,
                borderWidth: StyleSheet.hairlineWidth,
                borderColor: colors.border,
                backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
              })}
            >
              <Text variant="callout">{groupName[group]}</Text>
              <Icon name="chevron-forward" size={12} color={colors.textTertiary} />
            </Pressable>
          );
        })}
      </ScrollView>
      {/* The edge fade: the page's own colour over the chips' last few points, stronger to the edge. */}
      <View pointerEvents="none" style={styles.fade}>
        {[0.15, 0.35, 0.6, 0.85].map((opacity) => (
          <View key={opacity} style={{ flex: 1, backgroundColor: colors.background, opacity }} />
        ))}
      </View>
    </View>
  );
}

/** A suggestion or a result: its icon, its title with the match marked, its line, and where it lives. */
function ResultRow({ row, onPress }: { row: Row; onPress: () => void }) {
  const { colors, spacing } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${row.title}, ${row.line}, in ${pathOf(row.entry)}`}
      onPress={onPress}
      style={({ pressed }) => [
        styles.row,
        { gap: spacing.md, padding: spacing.md, backgroundColor: pressed ? colors.surfaceRaised : colors.surface },
      ]}
    >
      <Icon name={row.icon} color={colors.textSecondary} />
      <View style={styles.text}>
        <Text numberOfLines={2}>{marked(row, colors.accent)}</Text>
        <Text variant="footnote" tone="secondary" numberOfLines={2}>
          {row.line}
        </Text>
      </View>
      <Text variant="caption" tone="tertiary">
        {pathOf(row.entry)}
      </Text>
      <Icon name="chevron-forward" size={18} color={colors.textTertiary} />
    </Pressable>
  );
}

/** The title, its matched parts in the accent. */
function marked(row: Row, accent: string) {
  if (row.marks.length === 0) return row.title;
  const parts: ReactNode[] = [];
  let at = 0;
  for (const [start, end] of row.marks) {
    if (start < at) continue;
    parts.push(row.title.slice(at, start));
    parts.push(
      <Text key={start} weight="700" style={{ color: accent }}>
        {row.title.slice(start, end)}
      </Text>,
    );
    at = end;
  }
  parts.push(row.title.slice(at));
  return parts;
}

const styles = StyleSheet.create({
  search: { flexDirection: "column" },
  fade: { position: "absolute", right: 0, top: 0, bottom: 0, width: 32, flexDirection: "row" },
  row: { flexDirection: "row", alignItems: "center" },
  text: { flex: 1, gap: 2 },
});
