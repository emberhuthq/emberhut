import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { Pressable, View } from "react-native";
import { OWNER, type Issue, type TaskMoment } from "@/api";
import { emberCalled, emberName, shortEmberName } from "@/features/ember/name";
import { parseInstant, weekdayClock } from "@/format/time";
import { api } from "@/services";
import { useEmberhutState } from "@/state/queries";
import { useTheme } from "@/theme";
import { Card, Chip, Collapsible, Icon, Text, TextField } from "@/ui";
import { exchange, replyNote } from "./taskPage";

/**
 * Your reply on the task's own page: it goes where an answer goes (its worker, the session of its latest run, or the ember in
 * the task's room), never to Main. Above the box, your last message on the task and what came back since.
 */
export function TaskReply({ task, history, onSent }: { task: Issue; history: TaskMoment[]; onSent: () => void }) {
  const { colors, spacing } = useTheme();
  const [text, setText] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const called = shortEmberName(emberCalled(useEmberhutState().data), 16);
  const send = useMutation({
    mutationFn: (said: string) => api.tasks.message(task.id, { text: said, files: [] }),
    onSuccess: (result) => {
      setText("");
      setNote(replyNote(result));
      onSent();
    },
    onError: (error) => setNote(`Not sent: ${error.message}`),
  });
  const ready = text.trim() !== "" && !send.isPending;
  return (
    <Card>
      {exchange(history).map((moment, i) => (
        <Moment key={`${moment.at}-${i}`} moment={moment} />
      ))}
      <View style={{ flexDirection: "row", alignItems: "flex-end", gap: spacing.sm }}>
        <TextField
          value={text}
          onChangeText={setText}
          multiline
          placeholder={task.status === "waiting" ? "Answer in your own words…" : `Message ${called} about this task…`}
          accessibilityLabel="Your reply on the task"
          style={{ flex: 1 }}
        />
        <Pressable
          onPress={() => send.mutate(text.trim())}
          disabled={!ready}
          accessibilityRole="button"
          accessibilityLabel="Send reply"
          accessibilityState={{ disabled: !ready, busy: send.isPending }}
          hitSlop={6}
        >
          <Icon name="arrow-up-circle" size={36} color={ready ? colors.accent : colors.textTertiary} />
        </Pressable>
      </View>
      {note ? (
        <Text variant="footnote" tone="secondary">
          {note}
        </Text>
      ) : null}
    </Card>
  );
}

/** One step on the task: who took it, what it says, when. */
export function Moment({ moment }: { moment: TaskMoment }) {
  const { spacing } = useTheme();
  const at = parseInstant(moment.at);
  const mine = moment.who === OWNER;
  const ember = shortEmberName(emberName(useEmberhutState().data), 12);
  return (
    <View style={{ flexDirection: "row", gap: spacing.sm, alignItems: "flex-start" }}>
      <Chip label={mine ? "You" : ember} tone={mine ? "working" : "warning"} />
      <View style={{ flex: 1, gap: 2 }}>
        <Collapsible text={moment.text} lines={4} variant="callout" />
        {at ? (
          <Text variant="caption" tone="tertiary">
            {weekdayClock(at)}
          </Text>
        ) : null}
      </View>
    </View>
  );
}
