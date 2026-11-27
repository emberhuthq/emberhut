import type { AreaState, Issue } from "@/api";
import { useTheme, type Palette } from "@/theme";
import { Icon, type IconName } from "@/ui";
import { areaStateNames } from "./laneModel";
import { stateTone, statusName, taskState, type StateTone } from "./model";

const toneColor: Record<StateTone, keyof Palette> = {
  needsYou: "needsYou",
  working: "working",
  done: "done",
  danger: "danger",
  muted: "textTertiary",
};

const taskIcons: Record<string, IconName> = {
  waiting: "radio-button-on",
  running: "contrast",
  queued: "contrast-outline",
  scheduled: "time-outline",
  stalled: "pause-circle-outline",
  parked: "moon-outline",
  held: "hourglass-outline",
  done: "checkmark-circle",
  canceled: "archive-outline",
};

/** Where a task stands, as a small mark in its color: what needs you, a run going, done, nothing running. */
export function TaskStateMark({ task, size = 18 }: { task: Issue; size?: number }) {
  const { colors } = useTheme();
  const icon = taskIcons[taskState(task)] ?? "ellipse-outline";
  return <Icon name={icon} size={size} color={colors[toneColor[stateTone(task)]]} accessibilityLabel={statusName(task)} />;
}

const areaIcons: Record<AreaState | "scheduled", IconName> = {
  done: "checkmark-circle",
  working: "contrast",
  scheduled: "time-outline",
  needs_you: "radio-button-on",
  not_started: "ellipse-outline",
};

const areaTones: Record<AreaState | "scheduled", StateTone> = {
  done: "done",
  working: "working",
  scheduled: "working",
  needs_you: "needsYou",
  not_started: "muted",
};

/** A rolled-up state (an area, a step in its tree) as the same kind of mark. */
export function AreaStateMark({ state, size = 18 }: { state: AreaState | "scheduled"; size?: number }) {
  const { colors } = useTheme();
  return (
    <Icon
      name={areaIcons[state]}
      size={size}
      color={colors[toneColor[areaTones[state]]]}
      accessibilityLabel={state === "scheduled" ? "Scheduled" : areaStateNames[state]}
    />
  );
}
