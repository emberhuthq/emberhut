import { StyleSheet, View } from "react-native";
import type { ChatQuote } from "@/api";
import { useTheme } from "@/theme";
import { Text } from "@/ui";
import { plainText } from "@/ui/markdownParse";

/** The message a reply quotes: who said it and how it began. On your own bubble it takes the bubble's ink. */
export function QuoteView({ quote, onBubble = false }: { quote: ChatQuote; onBubble?: boolean }) {
  const { colors, spacing } = useTheme();
  const ink = onBubble ? colors.onBubbleMine : colors.textSecondary;
  const words = plainText(quote.text).trim() || (quote.file ? quote.file.name : "");
  return (
    <View style={[styles.quote, { gap: 1, paddingLeft: spacing.sm, borderLeftColor: onBubble ? colors.onBubbleMine : colors.accent }]}>
      <Text variant="caption" weight="600" style={{ color: ink }}>
        {quote.mine ? "You" : "Emberhut"}
      </Text>
      <Text variant="footnote" numberOfLines={2} style={{ color: ink, opacity: onBubble ? 0.85 : 1 }}>
        {words}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({ quote: { borderLeftWidth: 3 } });
