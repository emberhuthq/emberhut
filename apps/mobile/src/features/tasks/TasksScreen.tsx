import { useNavigation } from "@react-navigation/native";
import { useEffect, useLayoutEffect, useState } from "react";
import { Pressable, View } from "react-native";
import { taskKey, type Issue, type EmberhutState, type Project } from "@/api";
import { inBackground } from "@/log";
import type { TabScreenProps } from "@/navigation/types";
import { useActions, useConnection } from "@/state/hooks";
import { offlineReason } from "@/state/connection";
import { useApplySettings, useEmberhutState, useTasks } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, ErrorState, FoldRow, Icon, ListRow, Loading, Screen, Section, Text, TextField, usePullRefresh } from "@/ui";
import { AreaRow } from "./AreaRow";
import { flatTasks, subtasksOf, taskOverview } from "./model";
import { PlanTaskList } from "./PlanTaskList";
import { HealthChip, PriorityMark } from "./ProjectMarks";
import { TaskRow } from "./TaskRow";
import { useTaskPlans } from "./useTaskPlans";

/**
 * Your work by project, as the reference client's Projects list: Today (ranked by Emberhut), then each project with its areas and open
 * tasks, the projects with nothing open, tasks in no project, paused and archived projects, and your archived tasks
 * folded. What needs you has its own tab. A search runs over every task, done and archived ones too.
 *
 * While the server keeps task plans, Tasks is the work by where it stands, each task with its plan (PlanTaskList).
 */
export function TasksScreen({ navigation }: TabScreenProps<"Tasks">) {
  const { colors, spacing } = useTheme();
  const state = useEmberhutState();
  const pull = usePullRefresh(state.refetch);
  const offline = useConnection(offlineReason);
  const [query, setQuery] = useState("");
  const [adding, setAdding] = useState(false);
  const search = useDebounced(query.trim(), 300);
  const plans = useTaskPlans();
  const gone = useActions((s) => s.gone);

  useLayoutEffect(() => {
    navigation.setOptions({
      headerRight: () => (
        <Pressable
          onPress={() => setAdding((a) => !a)}
          hitSlop={12}
          accessibilityRole="button"
          accessibilityLabel="New task"
          style={{ paddingHorizontal: spacing.lg }}
        >
          <Icon name="add" size={26} color={colors.accent} />
        </Pressable>
      ),
    });
  }, [navigation, colors.accent, spacing.lg]);

  if (!state.data) {
    return (
      <Screen>
        {state.isError && offline ? (
          <ErrorState message={offline} onRetry={() => inBackground(state.refetch(), "TasksScreen: refetch")} />
        ) : (
          <Loading label="Reaching Emberhut" />
        )}
      </Screen>
    );
  }
  const top = (
    <>
      {adding && <NewTask hidden={state.data.ui?.hideProjects === true} onDone={() => setAdding(false)} />}
      <TextField
        value={query}
        onChangeText={setQuery}
        placeholder={`Find a task: ${taskKey(212)} or its title`}
        accessibilityLabel="Find a task"
        returnKeyType="search"
        autoCorrect={false}
      />
    </>
  );
  if (plans)
    return (
      <Screen padded={false} underBars>
        <PlanTaskList
          state={state.data}
          gone={gone}
          header={top}
          found={search ? <Found query={search} plans /> : null}
          footer={<ArchivedTasks plans />}
          refreshing={pull.refreshing}
          onRefresh={pull.onRefresh}
        />
      </Screen>
    );
  return (
    <Screen scroll {...pull}>
      {top}
      {search ? <Found query={search} /> : <Lists state={state.data} gone={gone} />}
    </Screen>
  );
}

/** The lists without plans: by project, or one list while projects are hidden. */
function Lists({ state, gone }: { state: EmberhutState; gone: ReadonlySet<number> }) {
  return state.ui?.hideProjects ? <FlatList state={state} gone={gone} /> : <ByProject state={state} gone={gone} />;
}

function ByProject({ state, gone }: { state: EmberhutState; gone: ReadonlySet<number> }) {
  const { spacing } = useTheme();
  const overview = taskOverview(state, gone);
  const name = (pid: number | null | undefined) => state.projects?.find((p) => p.id === pid)?.name;
  const row = (task: Issue, project?: string | null) => (
    <TaskRow key={task.id} task={task} project={project} subtasks={task.sub ? subtasksOf(task.id, state, gone) : undefined} />
  );
  return (
    <>
      <Section title="Today">{overview.today.map((t) => row(t, name(t.pid)))}</Section>
      {overview.groups.map((group) => (
        <View key={group.project.id} style={{ gap: spacing.sm }}>
          <ProjectHeader project={group.project} />
          <Section>
            {group.areas.map((area) => (
              <AreaRow key={`a${area.id}`} area={area} />
            ))}
            {group.tasks.map((t) => row(t))}
          </Section>
        </View>
      ))}
      <ProjectList title="Nothing open" projects={overview.clear} />
      <Section title="No project">{overview.loose.map((t) => row(t))}</Section>
      <ProjectList title="Paused" projects={overview.paused} />
      <ProjectList title="Archived" projects={overview.archived} />
      <ArchivedTasks />
    </>
  );
}

/** Projects hidden (state.ui): one list, no project headers. */
function FlatList({ state, gone }: { state: EmberhutState; gone: ReadonlySet<number> }) {
  const { needs, today, rest } = flatTasks(state, gone);
  const row = (task: Issue) => <TaskRow key={task.id} task={task} subtasks={task.sub ? subtasksOf(task.id, state, gone) : undefined} />;
  return (
    <>
      <Section title="Needs you" tone="needsYou">
        {needs.map(row)}
      </Section>
      <Section title="Today">{today.map(row)}</Section>
      <Section title={today.length === 0 && needs.length === 0 ? "Open" : "The rest"}>{rest.map(row)}</Section>
      {needs.length + today.length + rest.length === 0 && <Text tone="secondary">Nothing open.</Text>}
      <ArchivedTasks />
    </>
  );
}

/** A project's header in the list: its priority, name, health and what it finished this week; it opens the project. */
function ProjectHeader({ project }: { project: Project }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  return (
    <Pressable
      onPress={() => navigation.navigate("Project", { id: project.id })}
      accessibilityRole="button"
      accessibilityLabel={`${project.name} project`}
      style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm, paddingHorizontal: spacing.xs, paddingTop: spacing.sm }}
    >
      <PriorityMark priority={project.priority} />
      <Text variant="headline" style={{ flexShrink: 1 }} numberOfLines={1}>
        {project.name}
      </Text>
      <HealthChip health={project.health} />
      <View style={{ flex: 1 }} />
      {project.doneWeek > 0 ? (
        <Text variant="footnote" tone="secondary">
          {project.doneWeek} done
        </Text>
      ) : null}
      <Icon name="chevron-forward" size={16} color={colors.textTertiary} />
    </Pressable>
  );
}

function ProjectList({ title, projects }: { title: string; projects: Project[] }) {
  const navigation = useNavigation();
  return (
    <Section title={title}>
      {projects.map((p) => (
        <ListRow
          key={p.id}
          title={p.name}
          trailing={p.doneWeek > 0 ? <Text variant="footnote" tone="secondary">{`${p.doneWeek} done`}</Text> : undefined}
          onPress={() => navigation.navigate("Project", { id: p.id })}
        />
      ))}
    </Section>
  );
}

/** What you archived (put away for good), folded, from the server's whole list; each opens its page. */
export function ArchivedTasks({ plans = false }: { plans?: boolean }) {
  const [open, setOpen] = useState(false);
  const archived = useTasks({ archived: true, limit: 200 }, open);
  const gone = useActions((s) => s.gone);
  return (
    <Section>
      <FoldRow key="fold" title="Archived tasks" open={open} onToggle={() => setOpen(!open)} />
      {open && !archived.data ? <ListRow key="loading" title="Looking…" /> : null}
      {open && archived.data?.length === 0 ? <ListRow key="none" title="Nothing archived." /> : null}
      {open ? archived.data?.filter((t) => !gone.has(t.id)).map((t) => <TaskRow key={t.id} task={t} plans={plans} />) : null}
    </Section>
  );
}

/** A search over every task, filtered by the server. */
export function Found({ query, plans = false }: { query: string; plans?: boolean }) {
  const found = useTasks({ q: query, limit: 50 });
  const gone = useActions((s) => s.gone);
  const projects = useEmberhutState().data?.projects;
  if (found.isPending) return <Loading />;
  if (found.isError)
    return <ErrorState message={found.error.message} onRetry={() => inBackground(found.refetch(), "TasksScreen: refetch")} />;
  const tasks = found.data.filter((t) => !gone.has(t.id));
  if (tasks.length === 0) return <Text tone="secondary">No task matches “{query}”.</Text>;
  return (
    <Section title="Tasks">
      {tasks.map((t) => (
        <TaskRow key={t.id} task={t} project={projects?.find((p) => p.id === t.pid)?.name} plans={plans} />
      ))}
    </Section>
  );
}

/** A new task of yours; Emberhut puts it in a project. */
function NewTask({ hidden, onDone }: { hidden: boolean; onDone: () => void }) {
  const { spacing } = useTheme();
  const [title, setTitle] = useState("");
  const apply = useApplySettings();
  const ready = title.trim() !== "" && !apply.isPending;
  const add = () => {
    // The keyboard's Enter too: nothing to add, or one already on its way.
    if (!ready) return;
    apply.mutate(
      { taskCreate: title.trim() },
      {
        onSuccess: () => {
          setTitle("");
          onDone();
        },
      },
    );
  };
  return (
    <View style={{ gap: spacing.sm }}>
      <TextField
        value={title}
        onChangeText={setTitle}
        placeholder="What needs doing?"
        accessibilityLabel="New task's title"
        autoFocus
        onSubmitEditing={add}
      />
      <Text variant="footnote" tone="secondary">
        {hidden ? "It's yours." : "It's yours. Emberhut puts it in a project."}
      </Text>
      {apply.isError ? (
        <Text variant="footnote" tone="danger">
          Not added: {apply.error.message}
        </Text>
      ) : null}
      <View style={{ flexDirection: "row", gap: spacing.sm }}>
        <Button label="Add" onPress={add} disabled={!ready} loading={apply.isPending} />
        <Button label="Cancel" variant="plain" onPress={onDone} />
      </View>
    </View>
  );
}

function useDebounced(value: string, ms: number): string {
  const [shown, setShown] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setShown(value), value === "" ? 0 : ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return shown;
}
