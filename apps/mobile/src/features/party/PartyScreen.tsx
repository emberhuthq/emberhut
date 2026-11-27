import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  TextInput,
  View,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  type ScrollViewInstance,
} from "react-native";
import type { Party, PartyMessage, PartyModel, PartySpeaking } from "@/api";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { api } from "@/services";
import { useParties } from "@/state/queries";
import { useTheme } from "@/theme";
import { ErrorState, Icon, KeyboardAvoider, Loading, Screen, Text, type IconName } from "@/ui";
import { canStop, meter, modelName, personaColor, personaOf, resumeLabel, statusWord } from "./party";
import { Initial, StatusDot } from "./PartyParts";
import { useParty } from "./useParty";

/** One party, live: the cast, the talk as it happens with who is speaking word by word, and your controls. */
export function PartyScreen({ route, navigation }: RootScreenProps<"Party">) {
  const { id } = route.params;
  const { colors, spacing } = useTheme();
  const { view, ready, failed, retry, problem, act } = useParty(id);
  const models = useParties().data?.models ?? [];
  const party = view.party;
  const scroll = useRef<ScrollViewInstance>(null);
  const [following, setFollowing] = useState(true);

  const title = party?.title ?? route.params.title ?? "Party";
  const status = party?.status;
  useLayoutEffect(() => {
    navigation.setOptions({
      title,
      headerRight:
        status === undefined
          ? undefined
          : () => (
              <View style={[styles.row, { gap: spacing.lg }]}>
                {status === "running" ? (
                  <HeaderButton
                    icon="pause"
                    label="Pause after this turn"
                    color={colors.text}
                    onPress={() =>
                      inBackground(
                        act(() => api.parties.pause(id)),
                        "PartyScreen: act",
                      )
                    }
                  />
                ) : (
                  <HeaderButton
                    icon="play"
                    label={resumeLabel(status)}
                    color={colors.text}
                    onPress={() =>
                      inBackground(
                        act(() => api.parties.resume(id)),
                        "PartyScreen: act",
                      )
                    }
                  />
                )}
                {canStop(status) && (
                  <HeaderButton
                    icon="stop"
                    label="Stop now"
                    color={colors.danger}
                    onPress={() =>
                      inBackground(
                        act(() => api.parties.stop(id)),
                        "PartyScreen: act",
                      )
                    }
                  />
                )}
              </View>
            ),
    });
  }, [navigation, title, status, id, act, colors.text, colors.danger, spacing.lg]);

  const speaking = party?.speaking ?? null;
  useEffect(() => {
    if (following) scroll.current?.scrollToEnd({ animated: true });
  }, [following, view.messages.length, speaking?.text.length]);

  // Only your own scrolling stops the following; back at the bottom it follows again.
  const onScroll = (e: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
    setFollowing(contentOffset.y + layoutMeasurement.height >= contentSize.height - 80);
  };

  if (failed !== null && !ready) return <ErrorState message={`The party didn't load: ${failed}`} onRetry={retry} />;
  if (!ready || !party) return <Loading label="Opening the party" />;

  return (
    <Screen padded={false} edges={["bottom"]}>
      <KeyboardAvoider>
        <ScrollView
          ref={scroll}
          onScrollBeginDrag={() => setFollowing(false)}
          onMomentumScrollEnd={onScroll}
          onScrollEndDrag={onScroll}
          contentContainerStyle={{ padding: spacing.lg, gap: spacing.md }}
          keyboardShouldPersistTaps="handled"
        >
          <CastHeader party={party} models={models} />
          {view.messages.map((m) => (
            <Line key={m.id} line={m} party={party} />
          ))}
          {speaking && <Speaking speaking={speaking} party={party} />}
        </ScrollView>
        <Footer party={party} problem={problem} onSend={(text, steer) => act(() => api.parties.say(id, text, steer ? "steer" : "say"))} />
      </KeyboardAvoider>
    </Screen>
  );
}

function HeaderButton({ icon, label, color, onPress }: { icon: IconName; label: string; color: string; onPress: () => void }) {
  return (
    <Pressable onPress={onPress} accessibilityRole="button" accessibilityLabel={label} hitSlop={8}>
      <Icon name={icon} size={22} color={color} />
    </Pressable>
  );
}

/** Who is in it: each persona's colour, name and role, and their model. */
function CastHeader({ party, models }: { party: Party; models: PartyModel[] }) {
  const { colors, radius, scheme, spacing } = useTheme();
  return (
    <View style={{ gap: spacing.sm, padding: spacing.md, borderRadius: radius.md, backgroundColor: colors.surface }}>
      {party.personas.map((p) => (
        <View key={p.id} style={[styles.top, { gap: spacing.sm }]}>
          <View style={[styles.dot, { backgroundColor: personaColor(p.color, scheme) }]} />
          <Text variant="caption" numberOfLines={2} style={styles.fill}>
            <Text variant="caption" weight="600">
              {p.name}
            </Text>
            <Text variant="caption" tone="secondary">{` ${p.role}`}</Text>
          </Text>
          <Text variant="caption" tone="tertiary">
            {modelName(models, p.model)}
          </Text>
        </View>
      ))}
    </View>
  );
}

/** A line of the talk: a persona's on the left in their colour, your own on the right, your steer a pill in the middle. */
function Line({ line, party }: { line: PartyMessage; party: Party }) {
  const { colors, radius, scheme, spacing } = useTheme();
  if (line.kind === "steer")
    return (
      <View
        style={[
          styles.row,
          styles.pill,
          {
            gap: spacing.xs,
            paddingHorizontal: spacing.md,
            paddingVertical: 6,
            borderRadius: radius.pill,
            backgroundColor: `${colors.needsYou}1F`,
          },
        ]}
      >
        <Icon name="color-wand-outline" size={14} color={colors.needsYou} />
        <Text variant="caption" weight="500" selectable style={{ color: colors.needsYou, flexShrink: 1 }}>
          {line.text}
        </Text>
      </View>
    );
  if (line.kind === "say")
    return (
      <View style={[styles.mine, { paddingHorizontal: 13, paddingVertical: 9, borderRadius: 17, backgroundColor: colors.bubbleMine }]}>
        <Text selectable style={{ color: colors.onBubbleMine }}>
          {line.text}
        </Text>
      </View>
    );
  const color = personaColor(personaOf(party, line.who)?.color, scheme);
  return (
    <Turn name={line.name} color={color}>
      <Text selectable>{line.cut ? `${line.text} —` : line.text}</Text>
    </Turn>
  );
}

/** Who is speaking now, their words as they come. */
function Speaking({ speaking, party }: { speaking: PartySpeaking; party: Party }) {
  const { scheme } = useTheme();
  const color = personaColor(personaOf(party, speaking.who)?.color, scheme);
  return (
    <View accessibilityLabel={`${speaking.name} is speaking`}>
      <Turn name={speaking.name} color={color}>
        {speaking.text === "" ? <ActivityIndicator size="small" color={color} /> : <Text>{speaking.text}</Text>}
      </Turn>
    </View>
  );
}

function Turn({ name, color, children }: { name: string; color: string; children: ReactNode }) {
  const { spacing } = useTheme();
  return (
    <View style={[styles.top, { gap: 9, paddingRight: spacing.xl }]}>
      <Initial name={name} color={color} />
      <View style={[styles.fill, { gap: 3, alignItems: "flex-start" }]}>
        <Text variant="caption" weight="600" style={{ color }}>
          {name}
        </Text>
        <View style={{ paddingHorizontal: spacing.md, paddingVertical: spacing.sm, borderRadius: 15, backgroundColor: `${color}1A` }}>
          {children}
        </View>
      </View>
    </View>
  );
}

/** Where it stands, why it stopped, and your box: jump in (your words join the talk) or steer (a direction they all follow). */
function Footer({
  party,
  problem,
  onSend,
}: {
  party: Party;
  problem: string | null;
  onSend: (text: string, steer: boolean) => Promise<boolean>;
}) {
  const { colors, radius, spacing, type } = useTheme();
  const [text, setText] = useState("");
  const [steer, setSteer] = useState(false);
  const [sending, setSending] = useState(false);
  const words = text.trim();
  const running = party.status === "running";

  const send = () => {
    if (words === "" || sending) return;
    setSending(true);
    inBackground(
      onSend(words, steer)
        .then((ok) => {
          if (ok) setText((now) => (now.trim() === words ? "" : now));
        })
        .finally(() => setSending(false)),
      "PartyScreen: send",
    );
  };

  return (
    <View
      style={{
        gap: spacing.sm,
        paddingHorizontal: spacing.md,
        paddingTop: spacing.sm,
        paddingBottom: spacing.sm,
        backgroundColor: colors.surface,
      }}
    >
      <View style={[styles.row, { gap: 6 }]}>
        <StatusDot status={party.status} />
        <Text variant="caption" tone="secondary" numberOfLines={1} style={styles.fill}>
          <Text variant="caption" tone="secondary" weight="600">
            {statusWord(party.status)}
          </Text>
          {` · ${meter(party)}`}
        </Text>
      </View>
      {party.why && !running ? (
        <Text variant="caption" tone={party.status === "paused" ? "warning" : "secondary"}>
          {party.why}
        </Text>
      ) : null}
      {problem ? (
        <Text variant="caption" tone="warning">
          {problem}
        </Text>
      ) : null}
      <View style={[styles.row, styles.bottom, { gap: spacing.xs }]}>
        <Pressable
          onPress={() => setSteer((s) => !s)}
          accessibilityRole="switch"
          accessibilityState={{ checked: steer }}
          accessibilityLabel={
            steer
              ? "Steer: a direction everyone follows. Tap to jump in instead"
              : "Jump in: your words join the talk. Tap to steer instead"
          }
          style={styles.mode}
        >
          <Icon
            name={steer ? "color-wand-outline" : "chatbubble-outline"}
            size={22}
            color={steer ? colors.needsYou : colors.textSecondary}
          />
        </Pressable>
        <TextInput
          value={text}
          onChangeText={setText}
          placeholder={steer ? "Steer them, e.g. now argue the other side" : "Jump in, or name someone to ask them"}
          placeholderTextColor={colors.textTertiary}
          selectionColor={colors.accent}
          multiline
          style={[
            type.body,
            styles.input,
            {
              color: colors.text,
              backgroundColor: colors.surfaceRaised,
              borderRadius: radius.lg,
              paddingHorizontal: spacing.md,
              paddingVertical: spacing.sm,
            },
          ]}
          accessibilityLabel={steer ? "Steer them" : "Jump in"}
        />
        <Pressable
          onPress={send}
          disabled={words === "" || sending}
          accessibilityRole="button"
          accessibilityLabel={steer ? "Steer" : "Send"}
          style={[styles.mode, (words === "" || sending) && { opacity: 0.4 }]}
        >
          {sending ? <ActivityIndicator color={colors.accent} /> : <Icon name="arrow-up-circle" size={32} color={colors.accent} />}
        </Pressable>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  row: { flexDirection: "row", alignItems: "center" },
  top: { flexDirection: "row", alignItems: "flex-start" },
  bottom: { alignItems: "flex-end" },
  dot: { width: 9, height: 9, borderRadius: 5, marginTop: 4 },
  pill: { alignSelf: "center", maxWidth: "90%" },
  mine: { alignSelf: "flex-end", maxWidth: "80%", marginLeft: 48 },
  mode: { width: 40, height: 40, alignItems: "center", justifyContent: "center" },
  input: { flex: 1, minHeight: 40, maxHeight: 140, textAlignVertical: "center" },
});
