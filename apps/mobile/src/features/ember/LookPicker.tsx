import { useEffect, useMemo } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { Appearance, ThemeChange } from "@/api";
import { inBackground } from "@/log";
import { themeStore } from "@/services";
import { usePrefs, useThemeStore } from "@/state/hooks";
import { currentTheme } from "@/state/theme";
import { minTap, useTheme } from "@/theme";
import { Face, Icon, Segmented, Text } from "@/ui";
import { lookCards, lookChange, tintChange, tintChoices, type LookCard, type TintChoice } from "./look";

const appearances: readonly { value: Appearance; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

export interface LookPickerProps {
  /**
   * Where it shows: on Your ember's settings screen (the default: its parts headed, a line when Appearance's custom colour
   * is in use), or in onboarding's step (`setup`: no headings or notes, the cards smaller in a tight grid).
   */
  variant?: "settings" | "setup";
  /** Hears each pick once it's on its way (a pick shows at once, and works offline). */
  onPick?: (change: ThemeChange) => void;
}

/**
 * Your ember's look: Light, Dark or System, every preset the server lists as a small card in its own colours (its paper,
 * ink and accent in light, with its dark ground and accent in the corner), the one in use ticked, and the ember's face in
 * each tint the server lists. The same choice as Appearance's, kept on the server. It draws no title or frame of its own,
 * so Settings and onboarding put it where they need it.
 */
export function LookPicker({ variant = "settings", onPick }: LookPickerProps) {
  const { spacing } = useTheme();
  const setup = variant === "setup";
  const gap = setup ? spacing.xs + 2 : spacing.sm;
  const cache = usePrefs((s) => s.themeCache);
  const local = usePrefs((s) => s.theme);
  const page = useMemo(() => currentTheme({ themeCache: cache, theme: local }), [cache, local]);
  const error = useThemeStore((s) => s.error);

  useEffect(() => inBackground(themeStore.getState().refresh(), "LookPicker: refresh"), []);

  const pick = (change: ThemeChange) => {
    inBackground(themeStore.getState().choose(change), "LookPicker: choose");
    onPick?.(change);
  };
  const tints = tintChoices(page);
  return (
    <View style={{ gap: setup ? spacing.sm : spacing.md }}>
      <Segmented
        options={appearances}
        value={page.choice.appearance}
        onChange={(appearance) => pick({ appearance })}
        accessibilityLabel="Light or dark"
      />
      <View accessibilityRole="radiogroup" accessibilityLabel="Look" style={[styles.wrap, { gap }]}>
        {lookCards(page).map((card) => (
          <Card key={card.id} card={card} compact={setup} onPress={() => pick(lookChange(card.id))} />
        ))}
      </View>
      {!setup && page.choice.accent !== null ? (
        <Text variant="footnote" tone="secondary">
          A custom colour is in use (Appearance). Picking a look uses its own colour.
        </Text>
      ) : null}
      {tints.length > 0 ? (
        <View style={{ gap: spacing.sm }}>
          {setup ? null : (
            <Text variant="footnote" weight="600" tone="secondary">
              Your ember&apos;s colour
            </Text>
          )}
          <View accessibilityRole="radiogroup" accessibilityLabel="Your ember's colour" style={[styles.wrap, { gap }]}>
            {tints.map((tint) => (
              <TintButton key={tint.id} tint={tint} onPress={() => pick(tintChange(tint.id))} />
            ))}
          </View>
        </View>
      ) : null}
      {error ? (
        <Text variant="footnote" tone="warning">
          Not changed: {error}
        </Text>
      ) : null}
    </View>
  );
}

// A preset as a small page in its light colours: a card of ink lines and an accent button on its paper, its dark ground
// and accent in the corner, its name under it, ticked when it's in use.
// Compact (onboarding), smaller in a tight grid.
function Card({ card, compact, onPress }: { card: LookCard; compact: boolean; onPress: () => void }) {
  const { colors, radius, spacing } = useTheme();
  const { light, dark } = card;
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="radio"
      accessibilityState={{ selected: card.selected }}
      accessibilityLabel={card.isDefault ? `${card.name}, the default` : card.name}
      style={[
        styles.card,
        compact && styles.compact,
        {
          borderRadius: radius.md,
          padding: spacing.xs + 2,
          gap: spacing.xs + 2,
          borderColor: card.selected ? colors.accent : colors.border,
          backgroundColor: card.selected ? colors.accentSoft : "transparent",
        },
      ]}
    >
      <View style={[styles.page, compact && styles.compactPage, { backgroundColor: light.paper, borderColor: light.border }]}>
        <View style={[styles.sheet, { backgroundColor: light.surface }]}>
          <View style={[styles.line, styles.long, { backgroundColor: light.ink }]} />
          <View style={[styles.line, styles.short, { backgroundColor: light.muted }]} />
          <View style={[styles.button, { backgroundColor: light.accent }]} />
        </View>
        <View style={[styles.dark, { backgroundColor: dark.paper, borderColor: dark.border }]}>
          <View style={[styles.dot, { backgroundColor: dark.accent }]} />
        </View>
      </View>
      <View style={[styles.row, { gap: 3 }]}>
        {card.selected ? <Icon name="checkmark" size={14} color={colors.accent} /> : null}
        <Text variant="footnote" weight={card.selected ? "600" : "400"} numberOfLines={1} style={styles.shrink}>
          {card.name}
        </Text>
      </View>
    </Pressable>
  );
}

// A tint: the ember's face in it and its name, ringed when it's in use.
function TintButton({ tint, onPress }: { tint: TintChoice; onPress: () => void }) {
  const { colors, radius, spacing } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="radio"
      accessibilityState={{ selected: tint.selected }}
      accessibilityLabel={tint.isDefault ? `${tint.name} ember, the default` : `${tint.name} ember`}
      style={[
        styles.tint,
        {
          borderRadius: radius.md,
          paddingVertical: spacing.xs + 2,
          gap: spacing.xs,
          borderColor: tint.selected ? colors.accent : colors.border,
          backgroundColor: tint.selected ? colors.accentSoft : "transparent",
        },
      ]}
    >
      <Face size={40} tint={tint.id} />
      <Text variant="caption" weight={tint.selected ? "600" : "400"} tone={tint.selected ? "primary" : "secondary"}>
        {tint.name}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  wrap: { flexDirection: "row", flexWrap: "wrap" },
  row: { flexDirection: "row", alignItems: "center" },
  shrink: { flexShrink: 1 },
  card: { width: 104, borderWidth: 1 },
  compact: { width: 92 },
  compactPage: { height: 48 },
  page: { height: 58, borderRadius: 7, borderWidth: StyleSheet.hairlineWidth, padding: 7, overflow: "hidden" },
  sheet: { width: "70%", borderRadius: 5, padding: 6, gap: 4 },
  line: { height: 3, borderRadius: 2 },
  long: { width: "85%" },
  short: { width: "55%" },
  button: { marginTop: 2, height: 9, width: 26, borderRadius: 5 },
  dark: {
    position: "absolute",
    top: 6,
    right: 6,
    width: 20,
    height: 20,
    borderRadius: 5,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  dot: { width: 8, height: 8, borderRadius: 4 },
  tint: { width: 72, minHeight: minTap, borderWidth: 1, alignItems: "center" },
});
