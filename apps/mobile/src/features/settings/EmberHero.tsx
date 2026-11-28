import { useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { emberCalled, emberName, EMBER_NAME_MAX, leaving, nameCount, takesEdit } from "@/features/ember/name";
import { inBackground } from "@/log";
import { offlineReason } from "@/state/connection";
import { useConnection } from "@/state/hooks";
import { useApplySettings, useEmberhutState, useRenameEmber } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, Face, Icon, Text, TextField, Toggle } from "@/ui";
import { doingWord, type Now } from "./entries";

/** Where the name shows, said while it is typed. */
const WHERE = "Shows in the tab bar, chats, notifications and on your computer.";

/** How long "Saved" stays after a rename. */
const SAVED_FOR = 2000;

/**
 * Your ember's card: its face, its name as the title, what it is doing (`Watching · 1 job`) and what being on duty means,
 * with the On duty switch. A tap on the name (or opening the Name setting, `renaming`) edits it in place: Done or leaving
 * the field keeps it, Cancel keeps the old one, blank goes back to "Your ember". The new name shows at once everywhere;
 * one the server refuses goes back, with the reason under the title.
 */
export function EmberHero({ now, renaming = false }: { now: Now; renaming?: boolean }) {
  const { colors, radius, spacing } = useTheme();
  const state = useEmberhutState().data;
  const settings = useApplySettings();
  const rename = useRenameEmber();
  const offline = useConnection(offlineReason);
  const name = emberName(state);
  const custom = now.emberName;
  const [editing, setEditing] = useState(renaming);
  const [draft, setDraft] = useState(custom ?? "");
  const [problem, setProblem] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  // Cancel is pressed before the field loses its focus: leaving then keeps the old name.
  const cancelled = useRef(false);
  useEffect(() => {
    if (!saved) return;
    const timer = setTimeout(() => setSaved(false), SAVED_FOR);
    return () => clearTimeout(timer);
  }, [saved]);

  const begin = () => {
    cancelled.current = false;
    setDraft(custom ?? "");
    setProblem(null);
    setEditing(true);
  };
  const send = (typed: string) => {
    setSaved(false);
    inBackground(
      rename.mutateAsync(typed).then((outcome) => {
        if (outcome.ok) setSaved(true);
        else setProblem(`Not renamed: ${outcome.error}`);
      }),
      "EmberHero: rename",
    );
  };
  const leave = () => {
    if (!editing) return;
    if (cancelled.current) {
      setEditing(false);
      return;
    }
    const next = leaving(draft, custom);
    if (next.kind === "stay") {
      setProblem(`Not renamed: ${next.error}`);
      return;
    }
    setEditing(false);
    setProblem(null);
    if (next.kind === "rename") send(next.typed);
  };
  const count = editing ? nameCount(draft) : null;
  const status = offline ? `Can't reach ${emberCalled(state)}` : doingWord(now);

  return (
    <View style={{ gap: spacing.sm }}>
      <View
        style={[
          styles.card,
          { gap: spacing.md, padding: spacing.lg, borderRadius: radius.lg, backgroundColor: colors.surface, borderColor: colors.border },
        ]}
      >
        <Face size={44} />
        <View style={[styles.words, { gap: 2 }]}>
          {editing ? (
            <TextField
              value={draft}
              onChangeText={(text) => {
                if (!takesEdit(text)) return;
                setDraft(text);
                setProblem(null);
              }}
              onSubmitEditing={leave}
              onBlur={leave}
              autoFocus
              selectTextOnFocus
              placeholder="Your ember"
              autoCorrect={false}
              returnKeyType="done"
              accessibilityLabel="Name"
              accessibilityHint={WHERE}
            />
          ) : (
            <Pressable
              onPress={begin}
              disabled={offline !== null}
              accessibilityRole="button"
              accessibilityLabel={`Rename, now ${name}`}
              accessibilityHint={WHERE}
              hitSlop={6}
              style={({ pressed }) => [styles.title, { gap: spacing.xs }, pressed && styles.pressed]}
            >
              <Text variant="title" numberOfLines={2} style={styles.shrink}>
                {name}
              </Text>
              <Icon name="pencil" size={15} color={colors.textTertiary} />
            </Pressable>
          )}
          {editing ? (
            <View style={[styles.line, { gap: spacing.sm }]}>
              <Text variant="caption" tone="secondary" style={styles.shrink}>
                {WHERE}
              </Text>
              {count ? (
                <Text variant="caption" tone={count.startsWith(`${EMBER_NAME_MAX}/`) ? "warning" : "tertiary"}>
                  {count}
                </Text>
              ) : null}
            </View>
          ) : (
            <View style={[styles.line, { gap: spacing.xs }]}>
              <Text variant="footnote" tone={offline ? "warning" : "secondary"} numberOfLines={1} style={styles.shrink}>
                {status}
              </Text>
              {saved ? (
                <View style={[styles.line, { gap: 2 }]} accessibilityLiveRegion="polite">
                  <Icon name="checkmark" size={13} color={colors.done} />
                  <Text variant="footnote" tone="done">
                    Saved
                  </Text>
                </View>
              ) : null}
            </View>
          )}
          <Text variant="footnote" tone="tertiary">
            {now.onDuty
              ? "Works on its own, and answers when you message it."
              : "Stops working on its own. Still answers when you message it."}
          </Text>
        </View>
        <View style={styles.trailing}>
          <Toggle
            value={now.onDuty}
            onChange={(on) => settings.mutate({ on })}
            disabled={settings.isPending || offline !== null}
            label="On duty"
          />
        </View>
      </View>
      {problem ? (
        <Text variant="footnote" tone="warning" style={{ paddingHorizontal: spacing.xs }}>
          {problem}
        </Text>
      ) : null}
      {editing ? (
        <View style={styles.line}>
          <Button
            label="Cancel"
            variant="plain"
            onPressIn={() => {
              cancelled.current = true;
            }}
            onPress={() => {
              // It stays set until the next edit: the field's blur, if it comes after, keeps the old name too.
              cancelled.current = true;
              setEditing(false);
              setProblem(null);
            }}
          />
        </View>
      ) : custom !== undefined ? (
        <View style={styles.line}>
          <Button label="Use the default name" variant="plain" onPress={() => send("")} disabled={offline !== null || rename.isPending} />
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  card: { flexDirection: "row", alignItems: "center", borderWidth: 1 },
  words: { flex: 1, minWidth: 0 },
  title: { flexDirection: "row", alignItems: "center", alignSelf: "flex-start", maxWidth: "100%" },
  line: { flexDirection: "row", alignItems: "center" },
  shrink: { flexShrink: 1 },
  pressed: { opacity: 0.5 },
  trailing: { flexShrink: 0 },
});
