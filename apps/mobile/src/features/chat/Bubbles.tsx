import { memo, useMemo, type ReactNode } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { ChatFile, ChatQuote, MessageRef, TaskStatus } from "@/api";
import { ServerImage } from "@/features/files/ServerImage";
import { useTheme } from "@/theme";
import { Markdown, Text } from "@/ui";
import { parseBlocks } from "@/ui/markdownParse";
import { Attachments } from "./Attachments";
import { QuoteView } from "./QuoteView";
import { RefChips } from "./RefChips";

export interface BubbleProps {
  text: string;
  files?: readonly ChatFile[] | null;
  reply?: ChatQuote | null;
  onLink: (url: string) => void;
  /** A long press: reply, copy. */
  onMenu?: () => void;
  /** A tap on the quote: up to the message it answers. */
  onQuote?: (id: number) => void;
}

const radius = 19;
const corner = 6;

function inlineImage(url: string, alt: string): ReactNode {
  return <ServerImage path={url} style={styles.inlineImage} resizeMode="contain" accessibilityLabel={alt || "Picture"} />;
}

/** Your words: in your color, on the right; the message it answers quoted on top. */
export const UserBubble = memo(function UserBubble({ text, files, reply, onLink, onMenu, onQuote }: BubbleProps) {
  const { colors, spacing } = useTheme();
  const words = text.trim() !== "";
  const rich = useMemo(() => parseBlocks(text).some((block) => block.kind !== "para"), [text]);
  return (
    <View style={[styles.mine, rich && styles.mineWide, { gap: spacing.xs }]}>
      {files && files.length > 0 && <Attachments files={files} align="right" />}
      {words || reply ? (
        <Pressable
          onLongPress={onMenu}
          delayLongPress={350}
          accessibilityHint={onMenu ? "Long press to reply or copy" : undefined}
          style={[
            styles.bubble,
            {
              gap: spacing.sm,
              backgroundColor: colors.bubbleMine,
              borderBottomRightRadius: corner,
              paddingHorizontal: spacing.md + 2,
              paddingVertical: spacing.sm + 1,
            },
          ]}
        >
          {reply && (
            <Pressable onPress={onQuote ? () => onQuote(reply.id) : undefined} accessibilityHint="Shows the message it replies to">
              <QuoteView quote={reply} onBubble />
            </Pressable>
          )}
          {words && <Markdown text={text} color={colors.onBubbleMine} breaks taskRefs onLink={onLink} image={inlineImage} />}
        </Pressable>
      ) : null}
    </View>
  );
});

export interface AssistantBubbleProps extends BubbleProps {
  refs?: readonly MessageRef[] | null;
  taskStatus: (id: number) => TaskStatus | undefined;
}

/**
 * Emberhut's answer: across the whole width with no bubble, as chat assistants commonly show answers, so a long one
 * reads like a page; what it's about as chips after it. Its height is its words' own: nothing sizes it but its content.
 */
export const AssistantBubble = memo(function AssistantBubble({ text, files, refs, onLink, onMenu, taskStatus }: AssistantBubbleProps) {
  const { spacing } = useTheme();
  return (
    <View style={[styles.answer, { gap: spacing.sm }]}>
      {files && files.length > 0 && <Attachments files={files} />}
      {text.trim() !== "" && (
        <Pressable onLongPress={onMenu} delayLongPress={350} accessibilityHint={onMenu ? "Long press to reply or copy" : undefined}>
          <Markdown text={text} split taskRefs onLink={onLink} image={inlineImage} />
        </Pressable>
      )}
      {refs && refs.length > 0 && <RefChips refs={refs} taskStatus={taskStatus} />}
    </View>
  );
});

/** "Today 09:41", "Yesterday 18:02", "Monday 08:15", "Thu 2 Oct 19:30": small and centered over a row. */
export function Stamp({ date, now = new Date() }: { date: Date; now?: Date }) {
  const time = date.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
  const day = dayWord(date, now);
  return (
    <Text variant="caption" tone="secondary" align="center" style={styles.stamp}>
      <Text variant="caption" tone="secondary" weight="600">
        {day}
      </Text>{" "}
      {time}
    </Text>
  );
}

function dayWord(date: Date, now: Date): string {
  const start = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((start(now) - start(date)) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 6) return date.toLocaleDateString("en-GB", { weekday: "long" });
  return date.toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "short" });
}

const styles = StyleSheet.create({
  mine: { alignSelf: "flex-end", alignItems: "flex-end", maxWidth: "80%" },
  // A bubble holding a list, a quote or code gets the full 80%: sized to its words, its rows would be measured at one
  // width and drawn at another, and lines would spill out of it.
  mineWide: { width: "80%", alignItems: "stretch" },
  answer: { alignSelf: "stretch" },
  bubble: { borderRadius: radius },
  stamp: { paddingTop: 10, paddingBottom: 2 },
  inlineImage: { width: 240, height: 180, borderRadius: 12 },
});
