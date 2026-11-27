import type { Project } from "@/api";
import { Chip, Text } from "@/ui";

export const priorityNames = ["No priority", "Urgent", "High", "Medium", "Low"] as const;

/** Reminders-style: !!! urgent, !! high, ! medium. */
export function PriorityMark({ priority }: { priority: number }) {
  const mark = ["", "!!!", "!!", "!"][priority] ?? "";
  if (!mark) return null;
  return (
    <Text
      variant="callout"
      weight="700"
      tone={priority === 1 ? "danger" : priority === 2 ? "accent" : "secondary"}
      accessibilityLabel={priorityNames[priority]}
    >
      {mark}
    </Text>
  );
}

/** From its hard deadlines: on track, at risk, late. */
export function HealthChip({ health }: { health: Project["health"] }) {
  if (health === "late") return <Chip label="Late" tone="danger" icon="ellipse" />;
  if (health === "at_risk") return <Chip label="At risk" tone="warning" icon="ellipse" />;
  if (health === "on_track") return <Chip label="On track" tone="done" icon="ellipse" />;
  return null;
}
