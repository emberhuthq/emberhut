import { useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import { ASSISTANT, OWNER, type SessionStep } from "@/api";
import { Attachments } from "@/features/chat/Attachments";
import { monoFont, useTheme } from "@/theme";
import { Collapsible, Icon, Text, type IconName } from "@/ui";
import { CodeBlock, DiffBlock, LongText } from "./Blocks";
import { brief, foldedLabel, insideItems, toolLine, whoSaid, type ToolCall } from "./transcript";

/** A message to it (yours, Emberhut's, the runner's) in a tinted box, or Claude's words. */
export function SaidView({ step, queued, onLink }: { step: SessionStep; queued: boolean; onLink: (url: string) => void }) {
  const { colors, radius, spacing } = useTheme();
  if (step.kind !== "user") return <LongText text={step.text ?? ""} onLink={onLink} />;
  const words = brief(step.text ?? "");
  const files = step.files ?? [];
  const showWords = words.trim() !== "" && !(files.length > 0 && words === "(see the files)");
  const mine = step.by === OWNER;
  return (
    <View style={{ gap: spacing.xs, padding: spacing.sm + 2, borderRadius: radius.md, backgroundColor: colors.surfaceRaised }}>
      <View style={[styles.row, { gap: spacing.sm }]}>
        <Text
          variant="caption"
          weight="600"
          style={{ color: mine ? colors.accent : step.by === ASSISTANT ? colors.working : colors.textSecondary }}
        >
          {whoSaid(step.by)}
        </Text>
        {queued && (
          <View style={[styles.row, { gap: 3 }]}>
            <Icon name="time-outline" size={12} color={colors.textSecondary} />
            <Text variant="caption" tone="secondary">
              Queued
            </Text>
          </View>
        )}
      </View>
      {showWords && <Collapsible text={words} variant="callout" />}
      {files.length > 0 && <Attachments files={files} />}
    </View>
  );
}

/** A turn's end ("Turn 3 · done"), or what happened to the session (moved account, restarted). */
export function NoteView({ step }: { step: SessionStep }) {
  const { colors, spacing } = useTheme();
  if (step.kind === "end") {
    return (
      <Text variant="caption" tone="tertiary">
        Turn {step.n ?? 0} · {step.outcome === "completed" ? "done" : (step.outcome ?? "")}
      </Text>
    );
  }
  const text = step.text ?? "";
  return (
    <View style={[styles.row, { gap: spacing.xs }]}>
      <Icon name={text.startsWith("Moved") ? "swap-horizontal" : "information-circle-outline"} size={14} color={colors.warning} />
      <Text variant="footnote" tone="warning" style={styles.fill}>
        {text}
      </Text>
    </View>
  );
}

const toolIcons: Record<string, IconName> = {
  Bash: "terminal-outline",
  Read: "document-text-outline",
  Edit: "pencil",
  Write: "pencil",
  MultiEdit: "pencil",
  NotebookEdit: "pencil",
  Grep: "search",
  Glob: "search",
  Agent: "people-outline",
  Task: "people-outline",
  WebSearch: "globe-outline",
  WebFetch: "globe-outline",
};

/** One call as a compact line; open, its input or diff, what it returned, and a helper agent's own steps. */
export function ToolLine({ tool, live, onLink }: { tool: ToolCall; live: boolean; onLink: (url: string) => void }) {
  const { colors, spacing } = useTheme();
  const [open, setOpen] = useState(false);
  const c = tool.call;
  if (c.kind === "thinking") return <ThoughtLine step={c} />;
  const failed = tool.result?.error === true;
  const helperCalls = tool.inside.filter((s) => s.kind === "tool").length;
  const line = toolLine(c);
  return (
    <View style={{ gap: spacing.xs + 2 }}>
      <Pressable
        onPress={() => setOpen((o) => !o)}
        accessibilityRole="button"
        accessibilityLabel={`${line}${failed ? ", failed" : ""}`}
        accessibilityState={{ expanded: open }}
        style={[styles.row, { gap: spacing.sm, minHeight: 28 }]}
      >
        <Icon name={toolIcons[c.name ?? ""] ?? "construct-outline"} size={14} color={colors.textSecondary} />
        <Text variant="caption" numberOfLines={open ? 4 : 1} style={[styles.fill, styles.mono]}>
          {line}
        </Text>
        {tool.result === null && live ? (
          <ActivityIndicator size="small" color={colors.accent} />
        ) : failed ? (
          <Icon name="close-circle" size={14} color={colors.danger} />
        ) : null}
        {helperCalls > 0 && (
          <Text variant="caption" tone="secondary">
            {helperCalls}
          </Text>
        )}
        <Icon name={open ? "chevron-up" : "chevron-down"} size={12} color={colors.textTertiary} />
      </Pressable>
      {open && (
        <View style={{ gap: spacing.xs + 2 }}>
          {c.diff ? <DiffBlock text={c.diff} /> : c.input ? <CodeBlock title="Input" text={c.input} /> : null}
          {tool.result?.text ? <CodeBlock title={failed ? "Error" : "Output"} text={tool.result.text} error={failed} /> : null}
          {tool.inside.length > 0 && <HelperSteps tool={tool} live={live} onLink={onLink} />}
        </View>
      )}
    </View>
  );
}

function HelperSteps({ tool, live, onLink }: { tool: ToolCall; live: boolean; onLink: (url: string) => void }) {
  const { colors, spacing } = useTheme();
  return (
    <View style={{ gap: spacing.xs, paddingLeft: spacing.md, borderLeftWidth: 2, borderLeftColor: colors.border }}>
      <Text variant="caption" tone="secondary" weight="600">
        {"Helper agent's steps"}
      </Text>
      {insideItems(tool).map((item, i) => {
        switch (item.kind) {
          case "said":
            return (
              <Text key={i} variant="caption" numberOfLines={8} selectable>
                {item.step.text ?? ""}
              </Text>
            );
          case "tools":
            return item.calls.map((call) => <ToolLine key={call.call.seq} tool={call} live={live} onLink={onLink} />);
          case "note":
            return <NoteView key={i} step={item.step} />;
          default:
            return null;
        }
      })}
    </View>
  );
}

/** A finished thinking block: "Thought for 6s"; open, what it thought, when it shared it. */
function ThoughtLine({ step }: { step: SessionStep }) {
  const { colors, spacing } = useTheme();
  const [open, setOpen] = useState(false);
  const has = (step.text ?? "").trim() !== "";
  const label = step.ms === undefined ? "Thought" : `Thought for ${Math.max(1, Math.round(step.ms / 1000))}s`;
  return (
    <View style={{ gap: spacing.xs }}>
      <Pressable
        onPress={has ? () => setOpen((o) => !o) : undefined}
        accessibilityRole={has ? "button" : undefined}
        accessibilityLabel={label}
        style={[styles.row, { gap: spacing.sm, minHeight: 28 }]}
      >
        <Icon name="bulb-outline" size={14} color={colors.textSecondary} />
        <Text variant="caption" tone="secondary" style={[styles.fill, styles.italic]}>
          {label}
        </Text>
        {has && <Icon name={open ? "chevron-up" : "chevron-down"} size={12} color={colors.textTertiary} />}
      </Pressable>
      {open && (
        <Text variant="caption" tone="secondary" selectable style={[styles.italic, { paddingLeft: spacing.xl }]}>
          {step.text}
        </Text>
      )}
    </View>
  );
}

/** A finished run of calls, folded: "12 steps · ran the tests · 1 failed"; open, each as its own line. */
export function FoldedTools({ calls, onLink }: { calls: ToolCall[]; onLink: (url: string) => void }) {
  const { colors, spacing } = useTheme();
  const [open, setOpen] = useState(false);
  const { label, last, failed } = foldedLabel(calls);
  return (
    <View style={{ gap: spacing.xs }}>
      <Pressable
        onPress={() => setOpen((o) => !o)}
        accessibilityRole="button"
        accessibilityLabel={[label, last, failed > 0 ? `${failed} failed` : null].filter(Boolean).join(", ")}
        accessibilityState={{ expanded: open }}
        style={[styles.row, { gap: spacing.sm, minHeight: 32 }]}
      >
        <Icon name="construct-outline" size={14} color={colors.textSecondary} />
        <Text variant="footnote" weight="600">
          {label}
        </Text>
        {last && (
          <Text variant="footnote" tone="secondary" numberOfLines={1} style={styles.shrink}>
            · {last}
          </Text>
        )}
        {failed > 0 && (
          <Text variant="footnote" tone="danger">
            · {failed} failed
          </Text>
        )}
        <Icon name={open ? "chevron-down" : "chevron-forward"} size={12} color={colors.textTertiary} />
      </Pressable>
      {open && (
        <View style={{ gap: 2, paddingLeft: spacing.xs }}>
          {calls.map((call) => (
            <ToolLine key={call.call.seq} tool={call} live={false} onLink={onLink} />
          ))}
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
  shrink: { flexShrink: 1 },
  mono: { fontFamily: monoFont },
  italic: { fontStyle: "italic" },
});
