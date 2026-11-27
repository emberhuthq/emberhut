import { Children, Fragment, isValidElement, type ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import { useTheme } from "@/theme";
import { Chip, type ChipTone } from "./Chip";
import { SectionHeading } from "./SectionHeading";
import { Text, type TextTone } from "./Text";

export interface SectionProps {
  title?: string;
  /** A count beside the title, in a capsule of the title's tone. */
  count?: number;
  tone?: TextTone;
  /** On the header's right: a small button. */
  action?: ReactNode;
  /** A line under the group. */
  footer?: string;
  children: ReactNode;
}

/** A titled group of rows on one rounded surface, a hairline between them (iOS's grouped list section). */
export function Section({ title, count, tone = "secondary", action, footer, children }: SectionProps) {
  const { colors, radius, spacing } = useTheme();
  const rows = Children.toArray(children).filter(isValidElement);
  if (rows.length === 0) return null;
  return (
    <View style={{ gap: spacing.sm }}>
      {title || action ? (
        <View style={[styles.header, { paddingHorizontal: spacing.xs, gap: spacing.sm }]}>
          {title ? (
            <SectionHeading tone={tone} style={styles.title}>
              {title}
            </SectionHeading>
          ) : null}
          {count !== undefined ? <Count n={count} tone={tone} label={title} /> : null}
          {action}
        </View>
      ) : null}
      <View style={{ backgroundColor: colors.surface, borderRadius: radius.lg, overflow: "hidden" }}>
        {rows.map((row, i) => (
          <Fragment key={row.key ?? i}>
            {i > 0 && <View style={{ height: StyleSheet.hairlineWidth, backgroundColor: colors.border, marginLeft: spacing.lg }} />}
            {row}
          </Fragment>
        ))}
      </View>
      {footer ? (
        <Text variant="footnote" tone="secondary" style={{ paddingHorizontal: spacing.xs }}>
          {footer}
        </Text>
      ) : null}
    </View>
  );
}

const chipTone: Partial<Record<TextTone, ChipTone>> = { needsYou: "needsYou", done: "done", danger: "danger", accent: "needsYou" };

function Count({ n, tone, label }: { n: number; tone: TextTone; label?: string }) {
  return <Chip label={String(n)} tone={chipTone[tone] ?? "muted"} accessibilityLabel={`${n} ${label ?? ""}`.trim()} />;
}

const styles = StyleSheet.create({
  header: { flexDirection: "row", alignItems: "center" },
  title: { flex: 1, paddingHorizontal: 0 },
});
