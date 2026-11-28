import { useNavigation } from "@react-navigation/native";
import { useEffect, useLayoutEffect, useState } from "react";
import { Pressable, View } from "react-native";
import { taskKey, type Issue, type EmberhutState, type Project, type RunnerSession } from "@/api";
import { RecentRow, RunningRow } from "@/features/sessions/SessionRows";
import { allSessions, isRunning, shortTitle } from "@/features/sessions/sessionList";
import { inBackground } from "@/log";
import type { TabScreenProps } from "@/navigation/types";
import { useActions, useConnection } from "@/state/hooks";
import { offlineReason } from "@/state/connection";
import { useApplySettings, useEmberhutState, useTasks } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, ErrorState, FoldRow, Icon, ListRow, Loading, Screen, Section, Text, TextField, usePullRefresh } from "@/ui";
import { AreaRow } from "./AreaRow";
import { flatTasks, subtasksOf, taskOverview } from "./model";
import { tasklessSessions } from "./plan";
import { planLists, quietMinutes } from "./planView";
import { PlanTaskRow } from "./PlanTaskRow";
import { HealthChip, PriorityMark } from "./ProjectMarks";
import { TaskRow } from "./TaskRow";
import { useTaskPlans } from "./useTaskPlans";

/**
 * Your work by project, as the reference client's Projects list: Today (ranked by Emberhut), then each project with its areas and open
 * tasks, the projects with nothing open, tasks in no project, paused and archived projects, and your archived tasks
 * folded. What needs you has its own tab. A search runs over every task, done and archived ones too.
 *
 * While the server keeps task plans, Tasks is the work by where it stands, each task with its plan (Lists).
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
  return (
    <Screen scroll {...pull}>
      {adding && <NewTask hidden={state.data.ui?.hideProjects === true} onDone={() => setAdding(false)} />}
      <TextField
        value={query}
        onChangeText={setQuery}
        placeholder={`Find a task: ${taskKey(212)} or its title`}
        accessibilityLabel="Find a task"
        returnKeyType="search"
        autoCorrect={false}
      />
      {search ? <Found query={search} /> : <Lists state={state.data} plans={plans} gone={gone} />}
    </Screen>
  );
}

/**
 * The lists; with plans, the work by where it stands: a line to what needs you (its own tab lists it), what runs (a
 * blocked task that runs among them, the sessions running with no task after), what is blocked, the rest open, then the
 * other sessions with no task, so nothing that ran leaves the list.
 */
function Lists({ state, plans, gone }: { state: EmberhutState; plans: boolean; gone: ReadonlySet<number> }) {
  const navigation = useNavigation();
  if (!plans) return state.ui?.hideProjects ? <FlatList state={state} gone={gone} /> : <ByProject state={state} gone={gone} />;
  const lists = planLists(state, gone);
  const sessions = tasklessSessions(state);
  const running = sessions.filter((s) => isRunning(s.status));
  const every = allSessions(state);
  const now = new Date();
  const name = (pid: number | null | undefined) => (state.ui?.hideProjects ? null : state.projects?.find((p) => p.id === pid)?.name);
  const row = (t: Issue) => <PlanTaskRow key={`t${t.id}`} task={t} project={name(t.pid)} quiet={quietMinutes(t, every, now)} />;
  const open = (s: RunnerSession) => navigation.navigate("Session", { id: s.id, title: shortTitle(s.title) });
  return (
    <>
      {lists.needs > 0 ? (
        <Section>
          <ListRow
            key="needs"
            icon="hand-left-outline"
            title={`${lists.needs} ${lists.needs === 1 ? "needs" : "need"} you`}
            onPress={() => navigation.navigate("Tabs", { screen: "NeedsYou" })}
          />
        </Section>
      ) : null}
      <Section title="Running" count={lists.running.length + running.length}>
        {lists.running.map(row)}
        {running.map((s) => (
          <RunningRow key={`s${s.id}`} session={s} onOpen={() => open(s)} />
        ))}
      </Section>
      <Section title="Blocked" count={lists.blocked.length} tone="danger">
        {lists.blocked.map(row)}
      </Section>
      <Section title="Open" count={lists.open.length}>
        {lists.open.map(row)}
      </Section>
      {lists.running.length + running.length + lists.blocked.length + lists.open.length === 0 ? (
        <Text tone="secondary">Nothing open.</Text>
      ) : null}
      <OtherSessions sessions={sessions.filter((s) => !isRunning(s.status))} />
      <ArchivedTasks />
    </>
  );
}

/** Sessions with no task that aren't running: waiting on you, or finished in the last day. Each opens its session. */
function OtherSessions({ sessions }: { sessions: RunnerSession[] }) {
  const navigation = useNavigation();
  return (
    <Section title="Sessions with no task" footer="Every session, with filters: More › Sessions.">
      {sessions.map((s) => (
        <RecentRow key={s.id} session={s} onOpen={() => navigation.navigate("Session", { id: s.id, title: shortTitle(s.title) })} />
      ))}
    </Section>
  );
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
function ArchivedTasks() {
  const [open, setOpen] = useState(false);
  const archived = useTasks({ archived: true, limit: 200 }, open);
  const gone = useActions((s) => s.gone);
  return (
    <Section>
      <FoldRow key="fold" title="Archived tasks" open={open} onToggle={() => setOpen(!open)} />
      {open && !archived.data ? <ListRow key="loading" title="Looking…" /> : null}
      {open && archived.data?.length === 0 ? <ListRow key="none" title="Nothing archived." /> : null}
      {open ? archived.data?.filter((t) => !gone.has(t.id)).map((t) => <TaskRow key={t.id} task={t} />) : null}
    </Section>
  );
}

/** A search over every task, filtered by the server. */
function Found({ query }: { query: string }) {
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
        <TaskRow key={t.id} task={t} project={projects?.find((p) => p.id === t.pid)?.name} />
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
