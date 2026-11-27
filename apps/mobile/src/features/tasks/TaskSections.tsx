import { useState, type ReactNode } from "react";
import { Alert, View } from "react-native";
import { OWNER, taskKey, type Issue, type EmberhutState, type TaskMoment } from "@/api";
import { parseDay, parseInstant, shortDate } from "@/format/time";
import type { RootScreenProps } from "@/navigation/types";
import { useApplySettings } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, Collapsible, FoldRow, ListRow, ProgressBar, Section, Text } from "@/ui";
import { countText, fraction, isMine, rollupSummary } from "./model";
import { LinkRow, TaskLinkRow } from "./TaskProgress";
import { Moment } from "./TaskReply";

// The parts of a task's page under its first card: subtasks, findings, what came since you were asked, details,
// the brief, history, and stopping its worker.

/** What backs a decision (reports, logs, research), folded under the answers, never above them. */
export function Folded({ title, children }: { title: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Section>
        <FoldRow title={title} open={open} onToggle={() => setOpen(!open)} />
      </Section>
      {open ? children : null}
    </>
  );
}

/** A parent: how far its subtasks are, and each one in order. */
export function Subtasks({ task, full, navigate }: { task: Issue; full: Issue | undefined; navigate: (id: number) => void }) {
  const { colors, spacing } = useTheme();
  if (!task.sub) return null;
  const kids = (full?.subtasks ?? []).filter((k) => k.status !== "canceled");
  return (
    <Section title="Subtasks">
      <View key="rollup" style={{ padding: spacing.lg, gap: spacing.sm, backgroundColor: colors.surface }}>
        <Text variant="callout" weight="600">
          {rollupSummary(task.sub)}
        </Text>
        <ProgressBar value={fraction(task) ?? 0} color={task.sub.waiting.length > 0 ? colors.needsYou : undefined} />
      </View>
      {kids.map((kid) => (
        <TaskLinkRow key={kid.id} task={kid} onPress={() => navigate(kid.id)} />
      ))}
    </Section>
  );
}

const landedNames: Record<string, string> = {
  landed: "Landed: the fix is in the code, checked",
  partly: "Partly landed: some of the fix is in the code",
  not_landed: "Not landed: checked, and the fix isn't in the code",
  superseded: "Superseded: later work replaced it",
  unchecked: "Not verified yet: nobody has checked the code for it",
};

/** A finding: whether it landed, the scan that found it, the task that fixes it. */
export function Finding({ task, navigate }: { task: Issue; navigate: (id: number) => void }) {
  if (task.kind !== "finding") return null;
  return (
    <Section title="Finding">
      <ListRow key="state" title={landedNames[task.landed ?? "unchecked"] ?? landedNames.unchecked!} />
      {task.parent != null ? (
        <ListRow key="found" title="Found by" subtitle={taskKey(task.parent)} onPress={() => navigate(task.parent!)} />
      ) : null}
      {task.fixedBy != null ? (
        <ListRow key="fixed" title="Fixed by" subtitle={taskKey(task.fixedBy)} onPress={() => navigate(task.fixedBy!)} />
      ) : null}
    </Section>
  );
}

/** A scan's findings, open ones first. */
export function Findings({ full, navigate }: { full: Issue | undefined; navigate: (id: number) => void }) {
  const found = full?.findings ?? [];
  const order = ["not_landed", "partly", "unchecked", "landed", "superseded"];
  const sorted = [...found].sort((a, b) => order.indexOf(a.landed ?? "unchecked") - order.indexOf(b.landed ?? "unchecked"));
  return (
    <Section title="Findings" count={found.length}>
      {sorted.map((f) => (
        <TaskLinkRow key={f.id} task={f} onPress={() => navigate(f.id)} />
      ))}
    </Section>
  );
}

/** What happened since you were asked, so an outdated ask shows itself, folded. */
export function SinceAsked({ history, asked, question }: { history: TaskMoment[]; asked?: string | null; question: string }) {
  const [open, setOpen] = useState(false);
  const from = (parseInstant(asked)?.getTime() ?? Infinity) + 5_000;
  const since = history.filter(
    (m) => (parseInstant(m.at)?.getTime() ?? 0) > from && m.text !== question && !m.text.startsWith("Re-checked") && m.who !== OWNER,
  );
  if (since.length === 0) return null;
  return (
    <Section>
      <FoldRow key="fold" title={`Since Emberhut asked · ${since.length}`} open={open} onToggle={() => setOpen(!open)} />
      {open ? <MomentList key="list" moments={since.slice(-4)} /> : null}
    </Section>
  );
}

const sizeNames: Record<string, string> = { S: "S · about an hour", M: "M · about 3 hours", L: "L · about a day", XL: "XL · several days" };

/** Where it belongs, its dates and size, ultra mode, and the threads, tickets and chats that move it, folded. */
export function Details({
  task,
  full,
  state,
  navigate,
}: {
  task: Issue;
  full: Issue | undefined;
  state: EmberhutState | undefined;
  navigate: RootScreenProps<"Task">["navigation"];
}) {
  const [open, setOpen] = useState(false);
  const parent = task.parent != null ? state?.issues?.find((t) => t.id === task.parent) : undefined;
  const project = state?.ui?.hideProjects ? undefined : state?.projects?.find((p) => p.id === task.pid);
  const decideBy = parseDay(task.decideBy);
  const own = parseDay(task.due);
  const deadline = parseDay(task.deadline);
  const count = countText(task);
  return (
    <Section>
      <FoldRow key="fold" title="Details" open={open} onToggle={() => setOpen(!open)} />
      {open && task.parent != null ? (
        <ListRow
          key="parent"
          title="Part of"
          subtitle={parent ? `${parent.key} ${parent.title}` : taskKey(task.parent)}
          onPress={() => navigate.push("Task", { id: task.parent! })}
        />
      ) : null}
      {open && count ? <ListRow key="count" title="Progress" subtitle={count} /> : null}
      {open ? <ListRow key="owner" title="Owner" subtitle={isMine(task) ? "You" : "Emberhut"} /> : null}
      {open && project ? (
        <ListRow key="project" title="Project" subtitle={project.name} onPress={() => navigate.navigate("Project", { id: project.id })} />
      ) : null}
      {open && task.status === "waiting" ? (
        <ListRow key="decide" title="Decide by" subtitle={decideBy ? shortDate(decideBy) : "This week, by default"} />
      ) : null}
      {open && (own ?? deadline) ? (
        <ListRow
          key="deadline"
          title="Deadline"
          subtitle={
            own
              ? `${shortDate(own)}${task.hard ? ", hard" : ""}`
              : `From the project: ${shortDate(deadline!)}${task.hardDeadline ? ", hard" : ""}`
          }
        />
      ) : null}
      {open ? <ListRow key="size" title="Size" subtitle={task.size ? (sizeNames[task.size] ?? task.size) : "Not sized"} /> : null}
      {open && !isMine(task) ? (
        <ListRow key="ultra" title="Ultra mode" subtitle={task.ultra ? "On: its runs split across many agents" : "Off"} />
      ) : null}
      {open ? (full?.links ?? []).map((link) => <LinkRow key={`${link.kind}:${link.ref}`} link={link} />) : null}
    </Section>
  );
}

/** Your notes on your task, or the brief Emberhut's worker works from (folded: it's the worker's, not yours to read). */
export function Brief({ task }: { task: Issue }) {
  const { spacing } = useTheme();
  const [open, setOpen] = useState(false);
  const body = task.body?.trim();
  if (!body) return null;
  if (isMine(task)) {
    return (
      <Section title="Description">
        <View key="body" style={{ padding: spacing.lg }}>
          <Collapsible text={body} lines={8} />
        </View>
      </Section>
    );
  }
  return (
    <Section>
      <FoldRow key="fold" title="The brief Emberhut's worker works from" open={open} onToggle={() => setOpen(!open)} />
      {open ? (
        <View key="body" style={{ padding: spacing.lg }}>
          <Collapsible text={body} lines={12} />
        </View>
      ) : null}
    </Section>
  );
}

/** Every step, folded; the latest first. */
export function History({ history }: { history: TaskMoment[] }) {
  const [open, setOpen] = useState(false);
  if (history.length === 0) return null;
  return (
    <Section>
      <FoldRow key="fold" title={`History · ${history.length} steps`} open={open} onToggle={() => setOpen(!open)} />
      {open ? <MomentList key="list" moments={[...history].reverse()} /> : null}
    </Section>
  );
}

function MomentList({ moments }: { moments: TaskMoment[] }) {
  const { colors, spacing } = useTheme();
  return (
    <View style={{ padding: spacing.lg, gap: spacing.md, backgroundColor: colors.surface }}>
      {moments.map((m, i) => (
        <Moment key={`${m.at}-${i}`} moment={m} />
      ))}
    </View>
  );
}

/** Stops the worker's current run where it is, asked first. */
export function StopWorker({ task }: { task: Issue }) {
  const apply = useApplySettings();
  const confirm = () =>
    Alert.alert(`Stop Emberhut's worker on ${task.key}?`, "Its current run ends where it is.", [
      { text: "Cancel", style: "cancel" },
      { text: "Stop worker", style: "destructive", onPress: () => apply.mutate({ taskStop: task.id }) },
    ]);
  return (
    <Button
      label={apply.isSuccess ? "Worker stopped" : "Stop Emberhut's worker"}
      icon="stop-circle-outline"
      variant="destructive"
      onPress={confirm}
      loading={apply.isPending}
      disabled={apply.isSuccess}
    />
  );
}
