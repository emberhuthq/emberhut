import { useEffect, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import type { LiveAgent } from "@/api";
import { monoFont, useTheme } from "@/theme";
import { Icon, Text } from "@/ui";
import { Elapsed } from "./Elapsed";
import { FoldedTools, ToolLine } from "./Steps";
import { isAgentCall, tokenCount, type ToolCall } from "./transcript";

/** Pinned above the reply box while a turn runs: the word for what it's doing, the step it's on, how long, how many tokens. */
export function LiveBar({
  word,
  detail,
  since,
  tokens,
}: {
  word: (tick: number) => string;
  detail: string | null;
  since: string | null;
  tokens: number | null;
}) {
  const { colors, spacing } = useTheme();
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setTick((t) => t + 1), 3_500);
    return () => clearInterval(timer);
  }, []);
  const now = word(tick);
  return (
    <View
      accessible
      accessibilityLabel={`${now}${detail ? `, ${detail}` : ""}`}
      style={[
        styles.row,
        {
          gap: spacing.md,
          paddingHorizontal: spacing.lg,
          paddingVertical: spacing.sm,
          backgroundColor: colors.surface,
          borderTopColor: colors.border,
          borderTopWidth: StyleSheet.hairlineWidth,
        },
      ]}
    >
      <ActivityIndicator size="small" color={colors.accent} />
      <View style={styles.fill}>
        <Text variant="callout" weight="600" numberOfLines={1}>
          {now}…
        </Text>
        {detail ? (
          <Text variant="caption" tone="secondary" numberOfLines={1} style={styles.mono}>
            {detail}
          </Text>
        ) : null}
      </View>
      <View style={styles.end}>
        <Elapsed since={since} tone="secondary" />
        {tokens !== null && tokens > 0 && (
          <Text variant="caption" tone="tertiary">
            ↓ {tokenCount(tokens)} tokens
          </Text>
        )}
      </View>
    </View>
  );
}

/** Its thinking as it's written: the newest lines, under a quiet "Thinking" with its timer. */
export function ThinkingLive({ text, since }: { text: string; since: string | null }) {
  const { spacing } = useTheme();
  return (
    <View style={{ gap: spacing.xs }} accessible accessibilityLabel="Thinking">
      <View style={[styles.row, { gap: spacing.sm }]}>
        <Text variant="caption" tone="secondary" weight="600">
          Thinking
        </Text>
        <Elapsed since={since} tone="tertiary" />
      </View>
      {text.trim() !== "" && (
        <Text variant="caption" tone="secondary" numberOfLines={5} style={styles.italic}>
          {text.slice(-420)}
        </Text>
      )}
    </View>
  );
}

type Part = { kind: "one"; call: ToolCall } | { kind: "agents"; calls: ToolCall[] } | { kind: "folded"; calls: ToolCall[] };

/** A running turn's calls: older ones folded into a count, the newest few as lines, sub-agents started together side by side. */
export function LiveTools({
  calls,
  agents,
  onAgent,
  onLink,
}: {
  calls: ToolCall[];
  agents: readonly LiveAgent[];
  onAgent: (call: ToolCall, name: string) => void;
  onLink: (url: string) => void;
}) {
  const { spacing } = useTheme();
  const keep = 4;
  const cut = Math.max(0, calls.length - keep);
  const parts: Part[] = cut > 0 ? [{ kind: "folded", calls: calls.slice(0, cut) }] : [];
  for (const call of calls.slice(cut)) {
    const last = parts.at(-1);
    if (isAgentCall(call.call.name)) {
      if (last?.kind === "agents") last.calls.push(call);
      else parts.push({ kind: "agents", calls: [call] });
    } else parts.push({ kind: "one", call });
  }
  return (
    <View style={{ gap: spacing.xs }}>
      {parts.map((p) =>
        p.kind === "folded" ? (
          <FoldedTools key={`f${p.calls[0]!.call.seq}`} calls={p.calls} onLink={onLink} />
        ) : p.kind === "one" ? (
          <ToolLine key={p.call.call.seq} tool={p.call} live onLink={onLink} />
        ) : (
          <AgentsRun key={`a${p.calls[0]!.call.seq}`} calls={p.calls} agents={agents} onAgent={onAgent} />
        ),
      )}
    </View>
  );
}

// Sub-agents side by side: each with its state, what it's on, its tools and tokens; a tap opens its own steps live.
function AgentsRun({
  calls,
  agents,
  onAgent,
}: {
  calls: ToolCall[];
  agents: readonly LiveAgent[];
  onAgent: (call: ToolCall, name: string) => void;
}) {
  const { colors, radius, spacing } = useTheme();
  const info = (c: ToolCall) => agents.find((a) => a.id === c.call.id);
  const running = (c: ToolCall) => info(c)?.running ?? c.result === null;
  const busy = calls.filter(running).length;
  return (
    <View style={{ gap: spacing.sm, padding: spacing.sm + 2, borderRadius: radius.md, backgroundColor: colors.surfaceRaised }}>
      {calls.length > 1 && (
        <Text variant="caption" tone="secondary" weight="600">
          {busy > 1
            ? `${busy} agents working in parallel`
            : busy === 1
              ? `1 of ${calls.length} agents working`
              : `${calls.length} agents done`}
        </Text>
      )}
      {calls.map((c) => {
        const a = info(c);
        const name = a?.description ?? c.call.summary ?? "Agent";
        const on = running(c);
        const tools = a?.tools ?? c.inside.filter((s) => s.kind === "tool").length;
        const now = c.inside.findLast((s) => s.kind === "tool")?.doing?.replace("Running: ", "$ ") ?? a?.last;
        const counts = [
          tools > 0 ? `${tools} tool${tools === 1 ? "" : "s"}` : null,
          (a?.tokens ?? 0) > 0 ? `${tokenCount(a?.tokens ?? 0)} tok` : null,
        ]
          .filter(Boolean)
          .join(" · ");
        return (
          <Pressable
            key={c.call.seq}
            onPress={c.call.id ? () => onAgent(c, name) : undefined}
            accessibilityRole="button"
            accessibilityLabel={`${name}, ${on ? "working" : "done"}${counts ? `, ${counts}` : ""}`}
            style={[styles.row, { gap: spacing.sm, alignItems: "flex-start" }]}
          >
            {on ? (
              <ActivityIndicator size="small" color={colors.accent} />
            ) : (
              <Icon
                name={c.result?.error ? "close-circle" : "checkmark-circle"}
                size={16}
                color={c.result?.error ? colors.danger : colors.done}
              />
            )}
            <View style={styles.fill}>
              <Text variant="caption" weight="600" numberOfLines={1}>
                {name}
              </Text>
              {on && now ? (
                <Text variant="caption" tone="secondary" numberOfLines={1} style={styles.mono}>
                  {now}
                </Text>
              ) : null}
            </View>
            <View style={styles.end}>
              {on && <Elapsed since={a?.started ?? c.call.at} tone="secondary" />}
              {counts !== "" && (
                <Text variant="caption" tone="tertiary">
                  {counts}
                </Text>
              )}
            </View>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
  end: { alignItems: "flex-end" },
  mono: { fontFamily: monoFont },
  italic: { fontStyle: "italic" },
});
