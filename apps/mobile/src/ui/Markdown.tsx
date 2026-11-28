import * as Clipboard from "expo-clipboard";
import { memo, useMemo, useState, type ReactNode } from "react";
import { Linking, Text as NativeText, Pressable, ScrollView, StyleSheet, View, type TextStyle } from "react-native";
import { inBackground } from "@/log";
import { monoFont, useTheme, type TypeVariant } from "@/theme";
import { parseBlocks, parseInline, isRightToLeft, type Block, type Span } from "./markdownParse";
import { Text } from "./Text";

export interface MarkdownProps {
  text: string;
  variant?: TypeVariant;
  /** The words' color (your own bubble's text); the theme's text by default. */
  color?: string;
  /** Two sentences a paragraph (Emberhut's answers in the chat). */
  split?: boolean;
  /** A paragraph's lines stay on their own lines, as typed (your own messages). */
  breaks?: boolean;
  /** A task key (`taskKey(123)`) is a link to its task. */
  taskRefs?: boolean;
  /** A tapped link; the system opens it by default. */
  onLink?: (url: string) => void;
  /** A picture on its own line (![alt](url)); left out when not given. */
  image?: (url: string, alt: string) => ReactNode;
  selectable?: boolean;
}

/** Markdown as the chat shows it: blocks with air between them, quotes with their own Copy, tables as small cards. */
export const Markdown = memo(function Markdown({
  text,
  variant = "body",
  color,
  split = false,
  breaks = false,
  taskRefs = false,
  onLink = (url) => inBackground(Linking.openURL(url), "Markdown: openURL"),
  image,
  selectable = false,
}: MarkdownProps) {
  const theme = useTheme();
  const blocks = useMemo(() => parseBlocks(text, { split, breaks }), [text, split, breaks]);
  const ink = color ?? theme.colors.text;
  // On your own bubble links keep the bubble's ink, underlined; elsewhere the accent.
  const linkColor = color ?? theme.colors.accent;
  const inline = (s: string, style?: TextStyle) => (
    <Inline
      text={s}
      taskRefs={taskRefs}
      color={ink}
      linkColor={linkColor}
      variant={variant}
      onLink={onLink}
      selectable={selectable}
      style={style}
    />
  );
  return (
    <View style={{ gap: theme.spacing.sm }}>
      {blocks.map((block, i) => (
        <BlockView key={i} block={block} inline={inline} color={ink} variant={variant} image={image} />
      ))}
    </View>
  );
});

function BlockView({
  block,
  inline,
  color,
  variant,
  image,
}: {
  block: Block;
  inline: (text: string, style?: TextStyle) => ReactNode;
  color: string;
  variant: TypeVariant;
  image?: (url: string, alt: string) => ReactNode;
}) {
  const { colors, radius, spacing, type } = useTheme();
  switch (block.kind) {
    case "para":
      return inline(block.text, direction(block.text));
    case "heading":
      return inline(block.text, { fontWeight: block.level <= 2 ? "700" : "600", ...direction(block.text) });
    case "item":
      return (
        <View style={[styles.row, { gap: spacing.sm, paddingLeft: block.depth * 18 }]}>
          <Text variant={variant} weight="600" style={[styles.marker, { color: colors.textSecondary }]}>
            {block.marker}
          </Text>
          <View style={styles.fill}>{inline(block.text, direction(block.text))}</View>
        </View>
      );
    case "quote":
      return <Quote text={block.text} inline={inline} />;
    case "code":
      return (
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          style={{ backgroundColor: colors.surfaceRaised, borderRadius: radius.sm }}
        >
          <Text selectable style={[type.footnote, styles.mono, { color, padding: spacing.md }]}>
            {block.text}
          </Text>
        </ScrollView>
      );
    case "table":
      return <Table rows={block.rows} inline={inline} />;
    case "image":
      return image ? <>{image(block.url, block.alt)}</> : null;
    case "rule":
      return <View style={{ height: StyleSheet.hairlineWidth, backgroundColor: colors.border }} />;
  }
}

// A quote is a draft you may send as your own: its own Copy, the words only.
function Quote({ text, inline }: { text: string; inline: (text: string, style?: TextStyle) => ReactNode }) {
  const { colors, spacing } = useTheme();
  const [copied, setCopied] = useState(false);
  const copy = () => {
    inBackground(Clipboard.setStringAsync(text), "Markdown: setStringAsync");
    setCopied(true);
    setTimeout(() => setCopied(false), 2_000);
  };
  return (
    <View style={{ gap: spacing.xs, paddingLeft: spacing.md, borderLeftWidth: 3, borderLeftColor: colors.border }}>
      {inline(text, { color: colors.textSecondary, ...direction(text) })}
      <Pressable onPress={copy} accessibilityRole="button" accessibilityLabel="Copy the quote" hitSlop={8} style={styles.copy}>
        <Text variant="footnote" tone="accent" weight="600">
          {copied ? "Copied" : "Copy"}
        </Text>
      </Pressable>
    </View>
  );
}

// Each row a small card: its first cell the title, then "column: value" lines. It reads on a phone at any width.
function Table({ rows, inline }: { rows: string[][]; inline: (text: string, style?: TextStyle) => ReactNode }) {
  const { colors, spacing } = useTheme();
  const head = rows.length > 1 ? rows[0]! : [];
  const body = rows.length > 1 ? rows.slice(1) : rows;
  return (
    <View style={{ gap: spacing.sm }}>
      {body.map((row, r) => (
        <View
          key={r}
          style={{
            gap: 2,
            paddingTop: r > 0 ? spacing.sm : 0,
            borderTopWidth: r > 0 ? StyleSheet.hairlineWidth : 0,
            borderColor: colors.border,
          }}
        >
          {row[0] ? inline(row[0], { fontWeight: "600" }) : null}
          {row.slice(1).map((cell, c) => {
            if (cell === "") return null;
            const label = head[c + 1]
              ? parseInline(head[c + 1]!)
                  .map((s) => s.text)
                  .join("")
              : "";
            return <View key={c}>{inline(label ? `${label}: ${cell}` : cell)}</View>;
          })}
        </View>
      ))}
    </View>
  );
}

interface InlineProps {
  text: string;
  taskRefs: boolean;
  color: string;
  linkColor: string;
  variant: TypeVariant;
  onLink: (url: string) => void;
  selectable: boolean;
  style?: TextStyle;
}

function Inline({ text, taskRefs, color, linkColor, variant, onLink, selectable, style }: InlineProps) {
  const theme = useTheme();
  const spans = useMemo(() => parseInline(text, { taskRefs }), [text, taskRefs]);
  return (
    <Text variant={variant} selectable={selectable} style={[{ color }, style]}>
      {spans.map((span, i) => (
        <SpanText key={i} span={span} codeFill={theme.colors.surfaceRaised} linkColor={linkColor} onLink={onLink} />
      ))}
    </Text>
  );
}

// Nested in the paragraph's Text, so it inherits its size and color and only adds its own marks.
function SpanText({
  span,
  codeFill,
  linkColor,
  onLink,
}: {
  span: Span;
  codeFill: string;
  linkColor: string;
  onLink: (url: string) => void;
}) {
  const style: TextStyle = {
    ...(span.bold ? { fontWeight: "700" } : {}),
    ...(span.italic ? { fontStyle: "italic" } : {}),
    ...(span.code ? { fontFamily: monoFont, backgroundColor: codeFill } : {}),
    ...(span.link ? { color: linkColor, textDecorationLine: "underline" } : {}),
  };
  const link = span.link;
  if (!link) return <NativeText style={style}>{span.text}</NativeText>;
  return (
    <NativeText style={style} onPress={() => onLink(link)} accessibilityRole="link">
      {span.text}
    </NativeText>
  );
}

function direction(text: string): TextStyle {
  return isRightToLeft(text) ? { writingDirection: "rtl", textAlign: "right" } : {};
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "flex-start" },
  marker: { minWidth: 18, textAlign: "right" },
  fill: { flex: 1 },
  mono: { fontFamily: monoFont },
  copy: { alignSelf: "flex-start" },
});
