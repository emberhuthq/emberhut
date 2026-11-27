import { daysFromToday, deadlineText, parseDay, shortDate } from "@/format/time";
import { Chip } from "@/ui";

/**
 * A deadline as a chip: missed ones red, hard ones filled, soft ones plain. With tier (Needs you by project), the decide-by
 * group comes first: "Overdue · 2d", "Today", "This week · Fri", "This month · 3 Dec"; no date yet reads "This week".
 */
export function DeadlineChip({
  day,
  hard = false,
  decide = false,
  tier = false,
}: {
  day: string | null | undefined;
  hard?: boolean;
  decide?: boolean;
  tier?: boolean;
}) {
  const date = parseDay(day);
  if (!date) return tier ? <Chip label="This week" tone="muted" accessibilityLabel="Decide this week" /> : null;
  const days = daysFromToday(date);
  const text = deadlineText(date);
  const label =
    tier && days < 0
      ? `Overdue · ${-days}d`
      : tier && days > 0 && days <= 30
        ? `${days <= 6 ? "This week" : "This month"} · ${text}`
        : decide && days > 1
          ? `by ${text}`
          : text;
  return (
    <Chip
      label={label}
      tone={days < 0 ? "danger" : hard ? "warning" : decide && days === 0 ? "needsYou" : "muted"}
      icon={hard ? "flag" : undefined}
      accessibilityLabel={`${decide ? "Decide by" : hard ? "Hard deadline" : "Deadline"} ${shortDate(date)}${days < 0 ? ", late" : ""}`}
    />
  );
}
