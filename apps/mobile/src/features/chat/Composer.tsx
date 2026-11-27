import { useCallback, useState, type Dispatch, type SetStateAction } from "react";
import * as Haptics from "expo-haptics";
import { ActivityIndicator, Image, Pressable, ScrollView, StyleSheet, TextInput, View } from "react-native";
import type { ChatFile, ChatQuote } from "@/api";
import { inBackground } from "@/log";
import { useTheme } from "@/theme";
import { ActionSheet, Glass, Icon, Text, type IconName } from "@/ui";
import { appendSaid, sendAction, slopFor } from "./composerText";
import { QuoteView } from "./QuoteView";
import type { Upload } from "./uploads";
import { useDictation } from "./useDictation";
import { useUploads } from "./useUploads";

export interface ComposerProps {
  /** What's typed, held by the screen: a starter or a "talk about this" can write into it. */
  text: string;
  setText: Dispatch<SetStateAction<string>>;
  placeholder: string;
  /** You're answering this message: quoted above the box until you send. */
  replying?: ChatQuote | null;
  onUnquote?: () => void;
  /** The server can turn speech into text. */
  dictation?: boolean;
  /** Emberhut is answering: with nothing typed, the button stops it. */
  working?: boolean;
  onStop?: () => void;
  stopLabel?: string;
  /** A send that reports whether it went: the words and files stay in the box until it did. */
  onSend: (text: string, files: ChatFile[]) => void | Promise<boolean>;
  disabled?: boolean;
}

/**
 * The message box, as iOS's Messages has it: one rounded glass field that grows with the words, attach on its left, and
 * on its right dictation, always there (start typing, then switch to speaking), then send, or stop while Emberhut answers.
 * No band of its own around it.
 */
export function Composer({
  text,
  setText,
  placeholder,
  replying,
  onUnquote,
  dictation = false,
  working = false,
  onStop,
  stopLabel = "Stop Emberhut",
  onSend,
  disabled = false,
}: ComposerProps) {
  const { colors, radius, spacing, type } = useTheme();
  const [attaching, setAttaching] = useState(false);
  const uploads = useUploads();
  const voice = useDictation(useCallback((said: string) => setText((typed) => appendSaid(typed, said)), [setText]));
  const action = sendAction({ text, files: uploads.files.length, sending: uploads.sending, working: working && onStop !== undefined });

  const send = () => {
    if (action === "stop") {
      onStop?.();
      return;
    }
    if (action !== "send") return;
    const words = text.trim();
    const sentIds = uploads.uploads.filter((u) => u.status === "sent").map((u) => u.id);
    // Only what went: words typed and files picked while a slow send was on its way stay in the box.
    const clear = () => {
      setText((now) => (now.trim() === words ? "" : now));
      sentIds.forEach(uploads.remove);
      voice.discard();
    };
    inBackground(Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light), "Composer: haptic");
    const sent = onSend(words, uploads.files);
    if (sent instanceof Promise)
      inBackground(
        sent.then((ok) => ok && clear()),
        "Composer: send",
      );
    else clear();
  };

  return (
    <View style={[styles.dock, { gap: spacing.xs + 2 }]}>
      {replying && (
        <View style={[styles.row, { gap: spacing.sm }]}>
          <View style={styles.fill}>
            <QuoteView quote={replying} />
          </View>
          <IconButton icon="close" label="Don't reply to it" onPress={() => onUnquote?.()} small />
        </View>
      )}
      {uploads.uploads.length > 0 && <UploadStrip uploads={uploads.uploads} onRemove={uploads.remove} />}
      {voice.error && <ErrorLine text={voice.error} onHide={voice.clearError} />}
      {uploads.error && <ErrorLine text={uploads.error} onHide={uploads.clearError} />}
      <Glass style={[styles.field, { borderRadius: radius.lg + 4 }]}>
        <IconButton icon="add" label="Attach a picture or file" onPress={() => setAttaching(true)} disabled={disabled} />
        <TextInput
          value={text}
          onChangeText={setText}
          placeholder={voice.listening ? "Listening… tap the mic to stop" : placeholder}
          placeholderTextColor={colors.textTertiary}
          multiline
          editable={!disabled}
          style={[type.body, styles.input, { color: colors.text }]}
          accessibilityLabel={placeholder}
        />
        {dictation && (
          <Pressable
            onPress={voice.toggle}
            disabled={disabled || voice.preparing}
            accessibilityRole="button"
            accessibilityLabel={voice.listening ? "Stop dictation" : voice.preparing ? "Turning your words into text" : "Dictate"}
            hitSlop={inlineSlop}
            style={({ pressed }) => [styles.inline, pressed && styles.pressed]}
          >
            {voice.preparing ? (
              <ActivityIndicator color={colors.textSecondary} />
            ) : (
              <Icon name={voice.listening ? "stop-circle" : "mic-outline"} color={voice.listening ? colors.danger : colors.textSecondary} />
            )}
          </Pressable>
        )}
        {action !== "none" && <SendButton action={action} onPress={send} stopLabel={stopLabel} disabled={disabled} />}
      </Glass>
      <ActionSheet
        visible={attaching}
        actions={[
          { label: "Photo or video", icon: "images-outline", onPress: () => inBackground(uploads.pickMedia(), "Composer: pickMedia") },
          { label: "File", icon: "document-outline", onPress: () => inBackground(uploads.pickFile(), "Composer: pickFile") },
        ]}
        onClose={() => setAttaching(false)}
      />
    </View>
  );
}

/** Why dictation or a picker failed, above the box; a tap hides it. */
function ErrorLine({ text, onHide }: { text: string; onHide: () => void }) {
  return (
    <Pressable onPress={onHide} accessibilityRole="button" accessibilityLabel={`${text} Tap to hide.`}>
      <Text variant="footnote" tone="danger">
        {text}
      </Text>
    </Pressable>
  );
}

function SendButton({
  action,
  onPress,
  stopLabel,
  disabled,
}: {
  action: ReturnType<typeof sendAction>;
  onPress: () => void;
  stopLabel: string;
  disabled: boolean;
}) {
  const { colors } = useTheme();
  const stop = action === "stop";
  const off = disabled || action === "none" || action === "wait";
  return (
    <Pressable
      onPress={onPress}
      disabled={off}
      accessibilityRole="button"
      accessibilityLabel={stop ? stopLabel : action === "wait" ? "Sending your files" : "Send"}
      accessibilityState={{ disabled: off }}
      hitSlop={inlineSlop}
      style={({ pressed }) => [
        styles.send,
        { backgroundColor: off ? colors.surfaceRaised : stop ? colors.danger : colors.primary },
        pressed && styles.pressed,
      ]}
    >
      {action === "wait" ? (
        <ActivityIndicator color={colors.textSecondary} />
      ) : (
        <Icon name={stop ? "stop" : "arrow-up"} size={18} color={off ? colors.textTertiary : stop ? colors.onStatus : colors.onPrimary} />
      )}
    </Pressable>
  );
}

function IconButton({
  icon,
  label,
  onPress,
  disabled,
  small,
}: {
  icon: IconName;
  label: string;
  onPress: () => void;
  disabled?: boolean;
  small?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={({ pressed }) => [small ? undefined : styles.inline, pressed && styles.pressed]}
      hitSlop={small ? smallSlop : inlineSlop}
    >
      <Icon name={icon} size={small ? 18 : 24} color={disabled ? colors.textTertiary : colors.textSecondary} />
    </Pressable>
  );
}

// What's attached, small, above the box: a spinner while it goes up, a mark if it failed, × to take it off.
function UploadStrip({ uploads, onRemove }: { uploads: readonly Upload[]; onRemove: (id: string) => void }) {
  const { colors, radius, spacing } = useTheme();
  return (
    <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ gap: spacing.sm }}>
      {uploads.map((u) => (
        <View key={u.id} style={[styles.thumb, { borderRadius: radius.sm, backgroundColor: colors.surfaceRaised }]}>
          {u.kind === "image" ? (
            <Image source={{ uri: u.uri }} style={StyleSheet.absoluteFill} />
          ) : (
            <View style={[styles.fill, styles.center, { padding: spacing.xs }]}>
              <Icon name={u.kind === "video" ? "videocam-outline" : "document-outline"} size={20} color={colors.textSecondary} />
              <Text variant="caption" numberOfLines={1} ellipsizeMode="middle">
                {u.name}
              </Text>
            </View>
          )}
          {u.status !== "sent" && (
            <View style={[StyleSheet.absoluteFill, styles.center, styles.veil]}>
              {u.status === "sending" ? <ActivityIndicator color="#FFFFFF" /> : <Icon name="warning-outline" color={colors.warning} />}
            </View>
          )}
          <Pressable
            onPress={() => onRemove(u.id)}
            accessibilityRole="button"
            accessibilityLabel={`Remove ${u.name}${u.status === "failed" ? `, ${u.error ?? "it didn't upload"}` : ""}`}
            hitSlop={removeSlop}
            style={[styles.remove, { backgroundColor: colors.surface }]}
          >
            <Icon name="close" size={14} color={colors.text} />
          </Pressable>
        </View>
      ))}
    </ScrollView>
  );
}

// Messages' measures: a 36-point field, 32-point inline buttons, a few points between the field and the keyboard. Each
// button's touch area reaches the app's minimum (minTap, 44 pt) through its hit slop, whatever it looks like.
const inlineSize = 32;
const inlineSlop = slopFor(inlineSize);
const smallSlop = slopFor(18);
const removeSize = 20;
const removeSlop = slopFor(removeSize);
const styles = StyleSheet.create({
  dock: { paddingHorizontal: 10, paddingTop: 6, paddingBottom: 6 },
  row: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
  center: { alignItems: "center", justifyContent: "center" },
  field: { flexDirection: "row", alignItems: "flex-end", minHeight: 40, paddingHorizontal: 4, paddingVertical: 4 },
  input: { flex: 1, maxHeight: 140, paddingTop: 6, paddingBottom: 6, paddingHorizontal: 4 },
  inline: { width: inlineSize, height: inlineSize, alignItems: "center", justifyContent: "center" },
  send: { width: inlineSize, height: inlineSize, borderRadius: inlineSize / 2, alignItems: "center", justifyContent: "center" },
  pressed: { opacity: 0.6 },
  thumb: { width: 64, height: 64, overflow: "hidden" },
  veil: { backgroundColor: "rgba(0,0,0,0.35)" },
  remove: {
    position: "absolute",
    top: 3,
    right: 3,
    width: removeSize,
    height: removeSize,
    borderRadius: removeSize / 2,
    alignItems: "center",
    justifyContent: "center",
  },
});
