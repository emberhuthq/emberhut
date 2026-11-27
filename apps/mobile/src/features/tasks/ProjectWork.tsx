import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { View } from "react-native";
import type { AreaTree, Project, WorkRow } from "@/api";
import { needsRows } from "@/features/needsYou/model";
import { useActions } from "@/state/hooks";
import { useEmberhutState, useTasks } from "@/state/queries";
import { useTheme } from "@/theme";
import { EmptyState, FoldRow, ListRow, Loading, ProgressBar, Screen, Section, Text, usePullRefresh } from "@/ui";
import { DeadlineChip } from "./DeadlineChip";
import { LaneSection } from "./Lanes";
import { projectLanes, projectProgress } from "./laneModel";
import { HealthChip, priorityNames } from "./ProjectMarks";

/** A project's page, or one area of it: its header, what needs you in it, then its work by area, done folded. */
export function ProjectWork({ project: id, area }: { project: number; area?: number }) {
  const { spacing } = useTheme();
  const state = useEmberhutState();
  const pull = usePullRefresh(state.refetch);
  const gone = useActions((s) => s.gone);
  const closed = useTasks({ project: id, closed: true, limit: 200 });
  const project = state.data?.projects?.find((p) => p.id === id);
  if (!state.data) return <Loading label="Reaching Emberhut" />;
  if (!project) return <EmptyState icon="folder-outline" title="Project removed" />;
  const lanes = projectLanes(state.data, id, { area, closed: closed.data, gone });
  const needs = needsRows(state.data, gone)
    .filter((r) => r.pid === id && (area === undefined || r.area === area))
    .sort((a, b) => (a.decideBy ?? "9").localeCompare(b.decideBy ?? "9") || (a.asked ?? "").localeCompare(b.asked ?? ""));
  const areas = (state.data.areas ?? []).filter((a) => a.pid === id);
  const quiet = area === undefined ? areas.filter((a) => a.status !== "active") : [];
  return (
    <Screen scroll {...pull}>
      {area === undefined && <ProjectTop project={project} areas={areas} />}
      {needs.length > 0 && (
        <Section title="Needs you" count={needs.length} tone="needsYou">
          {needs.map((row) => (
            <NeedsInboxRow key={row.id} row={row} />
          ))}
        </Section>
      )}
      {lanes.map((lane) => (
        <LaneSection key={lane.key} lane={lane} />
      ))}
      {lanes.length === 0 && needs.length === 0 ? (
        <Text tone="secondary" style={{ paddingHorizontal: spacing.xs }}>
          Nothing open.
        </Text>
      ) : null}
      <QuietAreas areas={quiet} />
    </Screen>
  );
}

function ProjectTop({ project, areas }: { project: Project; areas: AreaTree[] }) {
  const { colors, spacing } = useTheme();
  const { done, total } = projectProgress(project, areas);
  return (
    <View style={{ gap: spacing.sm, paddingHorizontal: spacing.xs }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm, flexWrap: "wrap" }}>
        <HealthChip health={project.health} />
        {project.priority > 0 ? (
          <Text variant="footnote" tone="secondary">
            {priorityNames[project.priority]}
          </Text>
        ) : null}
        {project.kind !== "ongoing" && project.deadline ? <DeadlineChip day={project.deadline} hard={project.hard} /> : null}
        {project.status !== "active" ? (
          <Text variant="footnote" tone="secondary">
            {project.status === "paused" ? "Paused" : "Archived"}
          </Text>
        ) : null}
      </View>
      <ProgressBar value={done / Math.max(total, 1)} color={colors.done} label={`${done} of ${total} done`} />
      <Text variant="footnote" tone="secondary">
        {done} of {total} done
      </Text>
    </View>
  );
}

/** One thing that waits on you in the project: what you should do, the task under it, by when. */
function NeedsInboxRow({ row }: { row: WorkRow }) {
  const navigation = useNavigation();
  return (
    <ListRow
      icon="hand-left"
      title={row.action ?? row.ask ?? row.title}
      subtitle={row.title}
      trailing={<DeadlineChip day={row.decideBy} hard={row.hardBy === true} decide />}
      onPress={row.task != null ? () => navigation.navigate("Task", { id: row.task! }) : undefined}
    />
  );
}

/** Paused and done areas: one folded row, opening to their names. */
function QuietAreas({ areas }: { areas: AreaTree[] }) {
  const [open, setOpen] = useState(false);
  if (areas.length === 0) return null;
  return (
    <Section>
      <FoldRow
        key="fold"
        title={`${areas.length} paused or done area${areas.length === 1 ? "" : "s"}`}
        open={open}
        onToggle={() => setOpen(!open)}
      />
      {open ? areas.map((a) => <ListRow key={a.id} title={a.name} subtitle={a.status === "done" ? "Done" : "Paused"} />) : null}
    </Section>
  );
}
