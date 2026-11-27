import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { Appearance as AppearanceChoice, ThemePage } from "@/api";
import { inBackground } from "@/log";
import { themeStore } from "@/services";
import { offlineReason } from "@/state/connection";
import { useConnection, usePrefs, useThemeStore } from "@/state/hooks";
import { queryKeys } from "@/state/query";
import { currentTheme } from "@/state/theme";
import { minTap, useTheme } from "@/theme";
import { Button, Icon, Section, Segmented, Text, TextField } from "@/ui";
import { contrastLines, normalHex } from "./contrast";

const appearances: readonly { value: AppearanceChoice; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

/**
 * Emberhut's look, kept on the server so every app signed in to it follows one choice: light or dark, a preset's
 * colour, or a custom one held to AA. A preset works offline (sent when Emberhut's back); a custom colour needs the server.
 */
export function Appearance() {
  const { spacing } = useTheme();
  const cache = usePrefs((s) => s.themeCache);
  const local = usePrefs((s) => s.theme);
  const page = useMemo(() => currentTheme({ themeCache: cache, theme: local }), [cache, local]);
  const held = usePrefs((s) => s.themePending !== null);
  const error = useThemeStore((s) => s.error);
  const choose = themeStore.getState().choose;

  useEffect(() => inBackground(themeStore.getState().refresh(), "Appearance: refresh"), []);

  const footer = error
    ? `Not changed: ${error}`
    : held
      ? "Changed on this phone; it goes to Emberhut when it can be reached."
      : "The same in every Emberhut app signed in to this server.";
  return (
    <Section title="Appearance" footer={footer}>
      <View key="appearance" style={{ padding: spacing.md }}>
        <Segmented
          options={appearances}
          value={page.choice.appearance}
          onChange={(appearance) => inBackground(choose({ appearance }), "Appearance: choose")}
          accessibilityLabel="Appearance"
        />
      </View>
      <Presets key="presets" page={page} />
      {/* A new saved colour (here or from another device) starts the field again. */}
      <CustomAccent key={`custom:${page.choice.accent ?? ""}`} page={page} />
    </Section>
  );
}

// Each preset's colour in the appearance on screen; the chosen one ringed and ticked.
function Presets({ page }: { page: ThemePage }) {
  const { colors, spacing, scheme } = useTheme();
  const choose = themeStore.getState().choose;
  const chosen = page.choice.accent === null ? page.choice.preset : null;
  const name = page.presets.find((p) => p.id === chosen)?.name;
  return (
    <View style={{ gap: spacing.sm, padding: spacing.md }}>
      <View accessibilityRole="radiogroup" accessibilityLabel="Colour" style={[styles.row, styles.wrap, { gap: spacing.xs }]}>
        {page.presets.map((preset) => {
          const selected = preset.id === chosen;
          const tokens = preset[scheme];
          return (
            <Pressable
              key={preset.id}
              onPress={() => inBackground(choose({ preset: preset.id, accent: null }), "Appearance: choose")}
              accessibilityRole="radio"
              accessibilityState={{ selected }}
              accessibilityLabel={preset.name}
              style={[styles.ring, { borderColor: selected ? colors.text : "transparent" }]}
            >
              <View style={[styles.swatch, { backgroundColor: tokens.accent }]}>
                {selected ? <Icon name="checkmark" size={18} color={tokens.onAccent} /> : null}
              </View>
            </Pressable>
          );
        })}
      </View>
      <Text variant="footnote" tone="secondary">
        {name ?? (page.choice.accent ? `Custom colour ${page.choice.accent}` : "")}
      </Text>
    </View>
  );
}

// A hex of your own, previewed by the server as you type (its contrast, and what it had to change), saved on Apply.
function CustomAccent({ page }: { page: ThemePage }) {
  const { colors, radius, spacing, scheme } = useTheme();
  const offline = useConnection(offlineReason);
  const [applying, setApplying] = useState(false);
  const saved = page.choice.accent;
  const [draft, setDraft] = useState(saved ?? "");
  const typed = useDebounced(draft.trim(), 400);
  const hex = normalHex(typed);
  const preview = useQuery({
    queryKey: queryKeys.themePreview(hex ?? ""),
    queryFn: ({ signal }) => themeStore.getState().preview(hex!, signal),
    enabled: hex !== null && !offline,
    staleTime: Infinity,
    retry: false,
  });
  const shown = hex !== null ? preview.data : undefined;
  const tokens = shown?.tokens[scheme];
  const apply = () => {
    if (!hex || !shown) return;
    setApplying(true);
    inBackground(
      themeStore
        .getState()
        .choose({ accent: hex }, shown)
        .finally(() => setApplying(false)),
      "Appearance: choose",
    );
  };
  const clear = () => {
    setDraft("");
    if (saved !== null) inBackground(themeStore.getState().choose({ accent: null }), "Appearance: choose");
  };

  const note = offline
    ? "A custom colour needs Emberhut's server, which can't be reached now."
    : typed !== "" && hex === null
      ? "A hex colour like #3366FF or #36F."
      : preview.error
        ? `No preview: ${preview.error.message}`
        : null;
  return (
    <View style={{ gap: spacing.sm, padding: spacing.md }}>
      <Text variant="callout" weight="600">
        Custom colour
      </Text>
      <View style={[styles.row, { gap: spacing.sm }]}>
        <TextField
          value={draft}
          onChangeText={setDraft}
          placeholder="#3366FF"
          autoCapitalize="characters"
          autoCorrect={false}
          maxLength={7}
          accessibilityLabel="Custom colour, a hex code"
          style={styles.fill}
        />
        {tokens ? (
          <View
            accessibilityLabel={`Preview: ${tokens.accent}`}
            style={[styles.sample, { backgroundColor: tokens.accent, borderRadius: radius.md, paddingHorizontal: spacing.md }]}
          >
            <Text weight="600" style={{ color: tokens.onAccent }}>
              Aa
            </Text>
          </View>
        ) : null}
      </View>
      {note ? (
        <Text variant="footnote" tone={offline ? "secondary" : "warning"}>
          {note}
        </Text>
      ) : shown ? (
        contrastLines(shown, scheme).map((line) => (
          <Text key={line} variant="footnote" tone="secondary">
            {line}
          </Text>
        ))
      ) : null}
      <View style={[styles.row, { gap: spacing.sm }]}>
        <Button label="Apply" onPress={apply} disabled={!shown || offline !== null || hex === saved} loading={applying} />
        {saved !== null || draft !== "" ? <Button label="Clear" variant="plain" onPress={clear} /> : null}
      </View>
      {saved !== null ? (
        <View style={[styles.row, { gap: spacing.xs }]}>
          <Icon name="color-palette-outline" size={14} color={colors.textSecondary} />
          <Text variant="caption" tone="secondary">
            Clear goes back to the preset&apos;s own colour.
          </Text>
        </View>
      ) : null}
    </View>
  );
}

function useDebounced(value: string, ms: number): string {
  const [shown, setShown] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setShown(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return shown;
}

const swatch = 32;

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  wrap: { flexWrap: "wrap" },
  fill: { flex: 1 },
  // The ring around the chosen one, a gap between it and the colour; the whole of it a full tap target.
  ring: { width: minTap, height: minTap, borderRadius: minTap / 2, borderWidth: 2, alignItems: "center", justifyContent: "center" },
  swatch: { width: swatch, height: swatch, borderRadius: swatch / 2, alignItems: "center", justifyContent: "center" },
  sample: { height: minTap, alignItems: "center", justifyContent: "center" },
});
