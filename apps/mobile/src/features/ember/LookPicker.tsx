import { useEffect, useMemo } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { Appearance, ThemeChange } from "@/api";
import { inBackground } from "@/log";
import { themeStore } from "@/services";
import { usePrefs, useThemeStore } from "@/state/hooks";
import { currentTheme } from "@/state/theme";
import { minTap, useTheme } from "@/theme";
import { Face, Icon, type IconName, Segmented, Text } from "@/ui";
import { lookCards, lookChange, shownOf, tintChange, tintChoices, type LookCard, type Shown, type TintChoice } from "./look";
import { PagePicture, Swatch } from "./LookPicture";

const appearances: readonly { value: Appearance; label: string; icon: IconName }[] = [
  { value: "system", label: "System", icon: "phone-portrait-outline" },
  { value: "light", label: "Light", icon: "sunny-outline" },
  { value: "dark", label: "Dark", icon: "moon-outline" },
];

/** A card's width, and its page's (inside the card's padding and edge). */
const CARD = 104;
const CARD_PAGE = CARD - 2 * 7;

/** A swatch's circle, across (onboarding), and its button with the ring around it. */
const SWATCH = 28;
const SWATCH_BOX = SWATCH + 2 * (2 + 2);

export interface LookPickerProps {
  /**
   * Where it shows: on Your ember's settings screen (the default: Mode, Colours as cards and Your ember with each face
   * named, a line when Appearance's custom colour is in use), or in onboarding's step (`setup`, which has its own words:
   * no headings or notes, the modes as three icons, the presets as a row of split circles with the chosen one named, the
   * faces smaller and unnamed).
   */
  variant?: "settings" | "setup";
  /** Hears each pick once it's on its way (a pick shows at once, and works offline). */
  onPick?: (change: ThemeChange) => void;
}

/**
 * Your ember's look: Light, Dark or System; every preset the server lists as a small page in its own colours, in the
 * appearance chosen (both, split on a diagonal, for System), the one in use ticked; and the ember's face in each tint the
 * server lists. The same choice as Appearance's, kept on the server. It draws no title or frame of its own, so Settings and
 * onboarding put it where they need it.
 */
export function LookPicker({ variant = "settings", onPick }: LookPickerProps) {
  const { colors, spacing } = useTheme();
  const setup = variant === "setup";
  const cache = usePrefs((s) => s.themeCache);
  const local = usePrefs((s) => s.theme);
  const page = useMemo(() => currentTheme({ themeCache: cache, theme: local }), [cache, local]);
  const error = useThemeStore((s) => s.error);

  useEffect(() => inBackground(themeStore.getState().refresh(), "LookPicker: refresh"), []);

  const pick = (change: ThemeChange) => {
    inBackground(themeStore.getState().choose(change), "LookPicker: choose");
    onPick?.(change);
  };
  const cards = lookCards(page);
  const tints = tintChoices(page);
  const modes = (
    <Segmented
      options={setup ? appearances : appearances.map(({ value, label }) => ({ value, label }))}
      value={page.choice.appearance}
      onChange={(appearance) => pick({ appearance })}
      accessibilityLabel="Mode"
    />
  );
  const faces =
    tints.length > 0 ? (
      <View accessibilityRole="radiogroup" accessibilityLabel="Your ember" style={[styles.wrap, { gap: setup ? spacing.xs : spacing.sm }]}>
        {tints.map((tint) => (
          <TintButton key={tint.id} tint={tint} named={!setup} onPress={() => pick(tintChange(tint.id))} />
        ))}
      </View>
    ) : null;
  const problem = error ? (
    <Text variant="footnote" tone="warning">
      Not changed: {error}
    </Text>
  ) : null;

  if (setup) {
    return (
      <View style={{ gap: spacing.md }}>
        <View style={styles.modes}>{modes}</View>
        <View accessibilityRole="radiogroup" accessibilityLabel="Colours" style={[styles.wrap, styles.swatches, { gap: spacing.xs }]}>
          {cards.map((card) => (
            <SwatchButton key={card.id} card={card} onPress={() => pick(lookChange(card.id))} />
          ))}
        </View>
        {faces}
        {problem}
      </View>
    );
  }
  const shown = shownOf(page.choice.appearance);
  return (
    <View style={{ gap: spacing.md }}>
      <View style={[styles.row, { gap: spacing.sm }]}>
        <Label>Mode</Label>
        <View style={styles.grow}>{modes}</View>
      </View>
      <View style={{ gap: spacing.sm }}>
        <Label>Colours</Label>
        <View accessibilityRole="radiogroup" accessibilityLabel="Colours" style={[styles.wrap, { gap: spacing.sm }]}>
          {cards.map((card) => (
            <Card key={card.id} card={card} shown={shown} onPress={() => pick(lookChange(card.id))} />
          ))}
        </View>
        {page.choice.accent !== null ? (
          <Text variant="footnote" tone="secondary">
            A custom colour is in use (Appearance). Picking a look uses its own colour.
          </Text>
        ) : null}
      </View>
      {faces ? (
        <>
          <View style={{ height: StyleSheet.hairlineWidth, backgroundColor: colors.outline }} />
          <View style={{ gap: spacing.sm }}>
            <Label>Your ember</Label>
            {faces}
          </View>
        </>
      ) : null}
      {problem}
    </View>
  );
}

function Label({ children }: { children: string }) {
  return (
    <Text variant="footnote" weight="600" tone="secondary">
      {children}
    </Text>
  );
}

// A preset as a small page in its own colours, in the appearance shown, its name under it. The chosen one is the loud
// choice on the screen: accent edge, soft fill and a tick.
function Card({ card, shown, onPress }: { card: LookCard; shown: Shown; onPress: () => void }) {
  const { colors, radius } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="radio"
      accessibilityState={{ selected: card.selected }}
      accessibilityLabel={card.isDefault ? `${card.name}, the default` : card.name}
      style={[
        styles.card,
        {
          borderRadius: radius.md,
          borderColor: card.selected ? colors.accent : colors.outline,
          backgroundColor: card.selected ? colors.accentSoft : "transparent",
        },
      ]}
    >
      <View style={[styles.page, { borderColor: colors.outline }]}>
        <PagePicture light={card.light} dark={card.dark} shown={shown} width={CARD_PAGE} />
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

// A preset as onboarding's swatch: its paper and accent in the appearance in use, ringed and named when it's in use.
function SwatchButton({ card, onPress }: { card: LookCard; onPress: () => void }) {
  const { colors, scheme } = useTheme();
  const c = scheme === "dark" ? card.dark : card.light;
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="radio"
      accessibilityState={{ selected: card.selected }}
      accessibilityLabel={card.isDefault ? `${card.name}, the default` : card.name}
      style={styles.swatch}
    >
      <View style={[styles.ring, { borderColor: card.selected ? colors.accent : "transparent" }]}>
        <Swatch paper={c.paper} accent={c.accent} edge={colors.outline} size={SWATCH} />
      </View>
      {card.selected ? (
        <Text variant="caption" weight="600" numberOfLines={1} style={styles.swatchName}>
          {card.name}
        </Text>
      ) : null}
    </Pressable>
  );
}

// A tint: the ember's face in it, ringed in the accent when it's in use, and (on Settings) its name, bold when in use. No
// fill: the cards are the loud choice.
function TintButton({ tint, named, onPress }: { tint: TintChoice; named: boolean; onPress: () => void }) {
  const { colors, spacing } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="radio"
      accessibilityState={{ selected: tint.selected }}
      accessibilityLabel={tint.isDefault ? `${tint.name} ember, the default` : `${tint.name} ember`}
      style={({ pressed }) => [styles.tint, { gap: spacing.xs, opacity: pressed ? 0.7 : 1 }]}
    >
      <View style={[styles.ring, { borderColor: tint.selected ? colors.accent : "transparent" }]}>
        <Face size={named ? 36 : 32} tint={tint.id} />
      </View>
      {named ? (
        <Text variant="caption" weight={tint.selected ? "700" : "400"} tone={tint.selected ? "primary" : "secondary"}>
          {tint.name}
        </Text>
      ) : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  wrap: { flexDirection: "row", flexWrap: "wrap" },
  row: { flexDirection: "row", alignItems: "center" },
  grow: { flex: 1 },
  modes: { flexDirection: "row", width: 160 },
  shrink: { flexShrink: 1 },
  card: { width: CARD, borderWidth: 1, padding: 6, gap: 6 },
  page: { borderRadius: 7, borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  ring: { borderRadius: 999, borderWidth: 2, padding: 2 },
  // The chosen swatch's name hangs under it, centred, in room the row keeps below.
  swatches: { paddingBottom: 18 },
  swatch: { width: SWATCH_BOX, height: SWATCH_BOX, alignItems: "center", justifyContent: "center" },
  swatchName: { position: "absolute", top: SWATCH_BOX + 1, left: (SWATCH_BOX - 80) / 2, width: 80, textAlign: "center" },
  tint: { minWidth: minTap, minHeight: minTap, alignItems: "center", justifyContent: "center" },
});
