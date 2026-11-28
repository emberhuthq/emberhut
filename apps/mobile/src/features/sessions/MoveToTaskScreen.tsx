import { useState } from "react";
import { ActivityIndicator, View } from "react-native";
import { taskKey } from "@/api";
import type { RootScreenProps } from "@/navigation/types";
import { useActions } from "@/state/hooks";
import { useEmberhutState, useMoveSession, useTasks } from "@/state/queries";
import { useDebounced } from "@/features/tasks/useDebounced";
import { useTheme } from "@/theme";
import { ListRow, Screen, Section, Text, TextField } from "@/ui";
import { moveFailure, moveTargets } from "./moveToTask";
import { allSessions, shortTitle } from "./sessionList";

/**
 * Move to task…: the open tasks a session can work on instead of its own, found by key or title (the state's tasks at
 * once, then the server's search over every task). A tap moves it, then opens the task it moved to; a refusal shows the
 * server's reason.
 */
export function MoveToTaskScreen({ route, navigation }: RootScreenProps<"MoveToTask">) {
  const { session: id, title } = route.params;
  const { colors, spacing } = useTheme();
  const state = useEmberhutState();
  const gone = useActions((s) => s.gone);
  const move = useMoveSession();
  const [query, setQuery] = useState("");
  const search = useDebounced(query.trim(), 300);
  const found = useTasks({ q: search, limit: 50 }, search !== "");
  const session = allSessions(state.data).find((s) => s.id === id);
  const current = route.params.tasks ?? session?.tasks ?? [];
  const issues = state.data?.issues ?? [];
  const shown =
    search === ""
      ? moveTargets(issues, current, gone)
      : found.data
        ? moveTargets(found.data, current, gone)
        : moveTargets(issues, current, gone, search);
  const name = shortTitle(title ?? session?.title ?? id);
  const now = current.map((t) => issues.find((i) => i.id === t)?.key ?? taskKey(t)).join(", ");

  const pick = (task: number) => {
    if (move.isPending) return;
    move.mutate({ session: id, task }, { onSuccess: () => navigation.replace("Task", { id: task }) });
  };

  return (
    <Screen scroll>
      <View style={{ gap: spacing.sm }}>
        <Text variant="footnote" tone="secondary">
          {now
            ? `${name} works on ${now}. Its next turns work on the task you pick.`
            : `${name} works on no task. Its next turns work on the task you pick.`}
        </Text>
        <TextField
          value={query}
          onChangeText={setQuery}
          placeholder={`Find a task: ${taskKey(212)} or its title`}
          accessibilityLabel="Find a task"
          returnKeyType="search"
          autoCorrect={false}
          autoFocus
        />
        {move.isError ? (
          <Text variant="footnote" tone="danger">
            {moveFailure(move.error)}
          </Text>
        ) : null}
      </View>
      {shown.length === 0 ? (
        <Text tone="secondary">{search === "" ? "No open task to move it to." : `No open task matches “${search}”.`}</Text>
      ) : (
        <Section title={search === "" ? "Open tasks" : "Tasks"}>
          {shown.map((task) => (
            <ListRow
              key={task.id}
              title={task.title}
              subtitle={[task.key, task.project].filter(Boolean).join(" · ")}
              opens={false}
              trailing={move.isPending && move.variables.task === task.id ? <ActivityIndicator color={colors.accent} /> : undefined}
              onPress={() => pick(task.id)}
            />
          ))}
        </Section>
      )}
    </Screen>
  );
}
