import { useActions } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";
import { Loading, Screen, Section, Text, usePullRefresh } from "@/ui";
import { subtasksOf, todayLists } from "./model";
import { TaskRow } from "./TaskRow";

/** Today: what you have to decide today (the rest waits in Needs you), then the server's ranked list. */
export function TodayScreen() {
  const state = useEmberhutState();
  const pull = usePullRefresh(state.refetch);
  const gone = useActions((s) => s.gone);
  if (!state.data) return <Loading label="Reaching Emberhut" />;
  const name = (pid: number | null | undefined) => state.data.projects?.find((p) => p.id === pid)?.name;
  const { decide, ranked: today } = todayLists(state.data, gone);
  const hidden = state.data.ui?.hideProjects === true;
  return (
    <Screen scroll {...pull}>
      <Section title="To decide today" tone="needsYou">
        {decide.map((t) => (
          <TaskRow key={t.id} task={t} project={name(t.pid)} />
        ))}
      </Section>
      <Section
        title="Today"
        footer={
          hidden
            ? "Ranked by Emberhut: hard deadlines close to the work left first."
            : "Ranked by Emberhut: hard deadlines close to the work left first, then project priority."
        }
      >
        {today.map((t) => (
          <TaskRow key={t.id} task={t} project={name(t.pid)} subtasks={t.sub ? subtasksOf(t.id, state.data, gone) : undefined} />
        ))}
      </Section>
      {today.length === 0 && <Text tone="secondary">Nothing ranked for today.</Text>}
    </Screen>
  );
}
