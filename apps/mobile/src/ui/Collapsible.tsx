import { useState } from "react";
import { Pressable, View } from "react-native";
import { useTheme } from "@/theme";
import { Text, type TextTone } from "./Text";

export interface CollapsibleProps {
  text: string;
  /** Lines shown while folded. */
  lines?: number;
  tone?: TextTone;
  variant?: "body" | "callout" | "footnote";
}

/** Long text kept to a few lines, with Show more when there is more; selectable. */
export function Collapsible({ text, lines = 6, tone = "primary", variant = "body" }: CollapsibleProps) {
  const { spacing } = useTheme();
  const [open, setOpen] = useState(false);
  const long = text.length > lines * 55 || text.split("\n").length > lines;
  return (
    <View style={{ gap: spacing.xs }}>
      <Text variant={variant} tone={tone} numberOfLines={open || !long ? undefined : lines} selectable>
        {text}
      </Text>
      {long && (
        <Pressable onPress={() => setOpen(!open)} accessibilityRole="button" hitSlop={8}>
          <Text variant="footnote" weight="600" tone="accent">
            {open ? "Show less" : "Show more"}
          </Text>
        </Pressable>
      )}
    </View>
  );
}
