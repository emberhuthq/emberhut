import { memo } from "react";
import { StyleSheet, View } from "react-native";
import type { LiveAgent } from "@/api";
import { useTheme } from "@/theme";
import { LiveTools } from "./LiveParts";
import { QuestionCard } from "./QuestionCard";
import { FoldedTools, NoteView, SaidView } from "./Steps";
import { TodoList } from "./TodoList";
import { stepItems, type ToolCall, type Turn } from "./transcript";

export interface TurnViewProps {
  turn: Turn;
  /** The turn running now: its newest calls show as they happen; a finished one folds them. */
  live: boolean;
  /** Your message waits behind the running turn. */
  queued: boolean;
  /** The question it waits on you for, and your answer to it. */
  asking: number | null;
  onAnswer: (text: string) => void;
  agents: readonly LiveAgent[];
  onAgent: (call: ToolCall, name: string) => void;
  onLink: (url: string) => void;
}

/** A turn: what it was asked and said, the calls in between, its to-do list where it last changed, and its end. */
export const TurnView = memo(function TurnView({ turn, live, queued, asking, onAnswer, agents, onAgent, onLink }: TurnViewProps) {
  const { colors, spacing } = useTheme();
  const items = stepItems(turn.steps);
  const lastTools = items.findLastIndex((i) => i.kind === "tools");
  const lastTodos = items.findLastIndex((i) => i.kind === "todos");
  return (
    <View
      style={{
        gap: spacing.md,
        paddingVertical: spacing.sm,
        borderBottomWidth: StyleSheet.hairlineWidth,
        borderBottomColor: colors.border,
      }}
    >
      {items.map((item, i) => {
        switch (item.kind) {
          case "said":
            return <SaidView key={item.step.seq} step={item.step} queued={queued} onLink={onLink} />;
          case "tools":
            return live && i === lastTools ? (
              <LiveTools key={`t${item.calls[0]!.call.seq}`} calls={item.calls} agents={agents} onAgent={onAgent} onLink={onLink} />
            ) : (
              <FoldedTools key={`t${item.calls[0]!.call.seq}`} calls={item.calls} onLink={onLink} />
            );
          case "note":
            return <NoteView key={item.step.seq} step={item.step} />;
          case "question":
            return <QuestionCard key={item.step.seq} step={item.step} answer={item.step.seq === asking ? onAnswer : null} />;
          case "todos":
            // The running turn's list sits above the reply box instead.
            return !live && i === lastTodos && item.step.todos?.length ? <TodoList key={item.step.seq} todos={item.step.todos} /> : null;
        }
      })}
    </View>
  );
});
