import { useEffect, useLayoutEffect, useRef } from "react";
import { View, type ScrollViewInstance } from "react-native";
import { type Issue, type Plan, type TaskArtifact } from "@/api";
import { allSessions } from "@/features/sessions/sessionList";
import { mainRoom } from "@/config";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { actionsStore } from "@/services";
import { useEmberhutState, useTask } from "@/state/queries";
import { useTheme } from "@/theme";
import { Card, Collapsible, EmptyState, ListRow, Loading, Screen, Section, SectionHeading, Text, usePullRefresh } from "@/ui";
import { Activity } from "./Activity";
import { copyDraftFor } from "./copyDraft";
import { pagePlan, pageStamp, pageTask, stepLink } from "./plan";
import { PlanChecklist } from "./PlanChecklist";
import { PlanHeader } from "./PlanHeader";
import { quietMinutes, stepResults } from "./planView";
import { DecisionBlock, DecisionContext } from "./Decision";
import { standing, timeline } from "./taskPage";
import { leaveSoon, TaskActionsBar } from "./TaskActionsBar";
import { LiveCard, Results, RunList } from "./TaskProgress";
import { TaskReply } from "./TaskReply";
import { useTaskPlans } from "./useTaskPlans";

import { Brief, Details, Finding, Findings, Folded, History, SinceAsked, StopWorker, Subtasks } from "./TaskSections";
/**
 * A task's page, laid out as the reference client's: what you have to do first (the action, what to know, the draft, the answers and
 * your own reply), then the work itself (where it stands, what it made, its subtasks, its runs and sessions), then the rest
 * folded (details, the brief, history). The answers and actions stay pinned at the bottom.
 *
 * While the server keeps task plans: under the title what's done, now, next and what needs you (PlanHeader), the plan as
 * a checklist its agent ticks, then what needs you, your reply, the files and results (each step's first), then one
 * activity feed (turns, history, files, with the sessions one tap away) instead of where it stands, the runs, the
 * sessions and the history.
 */
export function TaskScreen({ route, navigation }: RootScreenProps<"Task">) {
  const { id } = route.params;
  const { spacing } = useTheme();
  const state = useEmberhutState();
  const fromState = state.data?.issues?.find((t) => t.id === id);
  const page = useTask(id);
  const pull = usePullRefresh(page.refetch);
  const scroll = useRef<ScrollViewInstance>(null);
  const askAt = useRef(0);
  const plans = useTaskPlans();
  const full = page.data ? pageTask(page.data) : undefined;
  const plan = plans ? pagePlan(page.data) : null;
  const { refetch } = page;
  // Asked again when the state says the task changed (with plans, also when its plan or its run moved); the first value
  // seen is what the page's own fetch already has.
  const updated = plans ? pageStamp(fromState) : fromState?.updated;
  const seen = useRef(updated);
  useEffect(() => {
    if (!updated || updated === seen.current) return;
    const first = seen.current === undefined;
    seen.current = updated;
    if (!first) inBackground(refetch(), "TaskScreen: refetch");
  }, [updated, refetch]);
  // The state's live fields; the page's own (history, runs, subtasks), and its artifacts when the state has none. The
  // page is fetched again whenever the state says the task changed.
  const task: Issue | undefined = fromState ? { ...fromState, artifacts: fromState.artifacts ?? full?.artifacts } : full;
  const key = task?.key;
  useLayoutEffect(() => {
    if (key) navigation.setOptions({ title: key });
  }, [navigation, key]);

  if (!task) {
    if (page.isError) return <EmptyState icon="document-text-outline" title="Task not found" message={page.error.message} />;
    return <Loading />;
  }
  const decision = task.status === "waiting" ? (task.decision ?? null) : null;
  const runs = timeline(full ?? task);
  const claimed = new Set((decision?.choices ?? []).flatMap((c) => c.media ?? []));
  const results = (task.artifacts ?? []).filter((a) => !claimed.has(a.id));
  const history = full?.history ?? [];
  const where = standing(full ?? task, runs);
  const answersInPage = (decision?.options.length ?? 0) > 0;
  const answer = (choice: string) => {
    copyDraftFor(choice, decision);
    actionsStore.getState().decide(task.id, choice);
    leaveSoon(navigation);
  };
  const room = state.data?.rooms?.find((r) => r.project != null && r.project === task.pid)?.id ?? mainRoom;
  // The page's own summary goes with its steps; the state's until the page answers.
  const summary = plans ? (plan?.summary ?? task.plan ?? null) : null;
  const shown: Plan | null = summary ? (plan ?? { steps: [], summary }) : null;
  const sessions = full?.sessions ?? [];
  const toAsk = decision ? () => scroll.current?.scrollTo({ y: Math.max(askAt.current - spacing.md, 0), animated: true }) : undefined;
  // Each step's result first, then what the task produced.
  const fromSteps = stepResults(shown).flatMap(({ step, link }): TaskArtifact[] => {
    const read = stepLink(link);
    if (!read || read.kind === "text") return [];
    return [
      {
        id: -step.id,
        kind: read.kind === "web" ? "link" : "file",
        name: `From: ${step.title}`,
        caption: read.kind === "server" && step.note ? step.note : read.text,
        url: link,
        at: step.finished ?? step.updated,
      },
    ];
  });
  return (
    <View style={{ flex: 1 }}>
      <Screen scroll {...pull} scrollRef={scroll}>
        <View style={{ gap: spacing.sm, paddingHorizontal: spacing.xs }}>
          <Text variant="title" selectable>
            {task.title}
          </Text>
          {shown ? (
            <PlanHeader
              task={task}
              plan={shown}
              sessions={sessions}
              quiet={quietMinutes(task, [...sessions, ...allSessions(state.data)])}
              onAsk={toAsk}
            />
          ) : (
            <LiveCard task={task} runs={runs} />
          )}
          {decision && !shown ? <DecisionContext decision={decision} decideBy={task.decideBy} /> : null}
        </View>
        {shown ? <PlanChecklist plan={shown} onAnswer={toAsk} /> : null}
        {decision && shown ? (
          <View style={{ gap: spacing.md }} onLayout={(e) => (askAt.current = e.nativeEvent.layout.y)}>
            <View style={{ paddingHorizontal: spacing.xs }}>
              <DecisionContext decision={decision} decideBy={task.decideBy} />
            </View>
            <DecisionBlock decision={decision} updated={task.updated} artifacts={task.artifacts ?? []} onAnswer={answer} />
          </View>
        ) : decision ? (
          <DecisionBlock decision={decision} updated={task.updated} artifacts={task.artifacts ?? []} onAnswer={answer} />
        ) : null}
        {task.status !== "canceled" ? (
          <View style={{ gap: spacing.sm }}>
            <SectionHeading>
              {!decision ? "Message" : decision.options.length === 0 ? "Your answer" : "Or answer in your own words"}
            </SectionHeading>
            <TaskReply
              task={task}
              history={history}
              onSent={() => setTimeout(() => inBackground(page.refetch(), "TaskScreen: refetch"), 1_000)}
            />
          </View>
        ) : null}
        {shown ? <Results items={[...fromSteps, ...results]} title="Files and results" /> : null}
        {!shown && decision && results.length > 0 ? (
          <Folded title={`Files and results · ${results.length}`}>
            <Results items={results} title="Files and results" />
          </Folded>
        ) : null}
        {where && !plans ? (
          <View style={{ gap: spacing.sm }}>
            <SectionHeading>Where it stands</SectionHeading>
            <Card>
              <Collapsible text={where.line} lines={4} />
              {where.left ? (
                <Text>
                  <Text tone="secondary">Left: </Text>
                  {where.left}
                </Text>
              ) : null}
            </Card>
          </View>
        ) : null}
        {!shown && !decision ? <Results items={results} /> : null}
        <Subtasks task={task} full={full} navigate={(kid) => navigation.push("Task", { id: kid })} />
        <Finding task={full ?? task} navigate={(other) => navigation.push("Task", { id: other })} />
        <Findings full={full} navigate={(other) => navigation.push("Task", { id: other })} />
        {plans ? (
          <Activity runs={runs} sessions={sessions} history={history} artifacts={[...fromSteps, ...results]} />
        ) : (
          <>
            {runs.length > 0 ? <RunList runs={runs} /> : null}
            <Section title="Sessions">
              {(full?.sessions ?? []).map((s) => (
                <ListRow
                  key={s.id}
                  icon="terminal-outline"
                  title={s.title}
                  subtitle={[s.status, s.place, s.doing].filter(Boolean).join(" · ")}
                  onPress={() => navigation.navigate("Session", { id: s.id, title: s.title })}
                />
              ))}
            </Section>
          </>
        )}
        {decision ? <SinceAsked history={history} asked={decision.asked} question={decision.question} /> : null}
        <Details task={task} full={full} state={state.data} navigate={navigation} />
        <Brief task={task} />
        {plans ? null : <History history={history} />}
        {task.status === "working" ? <StopWorker task={task} /> : null}
      </Screen>
      <TaskActionsBar task={task} issues={state.data?.issues ?? []} decision={decision} answersInPage={answersInPage} room={room} />
    </View>
  );
}
