import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Pressable, View } from "react-native";
import type { SwitchAction, SwitchPage } from "@/api";
import { api } from "@/services";
import { queryKeys } from "@/state/query";
import { useSwitches } from "@/state/queries";
import { useTheme, type Palette } from "@/theme";
import { Button, Icon, ListRow, Section, Text, Toggle } from "@/ui";

type Mcp = SwitchPage["mcp"][number];
type Skill = SwitchPage["skills"][number];

const mcpStatus: Record<Mcp["state"], string> = {
  connected: "Connected",
  failed: "Failed",
  "needs-auth": "Needs a sign-in",
  pending: "Connecting…",
  disabled: "Turned off in Claude Code",
  off: "Off",
  unknown: "Not loaded right now",
};

const mcpColor: Record<Mcp["state"], keyof Palette> = {
  connected: "done",
  failed: "danger",
  "needs-auth": "warning",
  pending: "warning",
  disabled: "textTertiary",
  off: "textTertiary",
  unknown: "textTertiary",
};

/**
 * Emberhut's MCP servers and skills, each with its state and your switch, which is real: Emberhut's sessions start again without
 * what you turn off. A tap on one shows the rest of it, and Reconnect for a server that isn't connected.
 */
export function Switches() {
  const client = useQueryClient();
  const page = useSwitches();
  const [open, setOpen] = useState<string | null>(null);
  const act = useMutation({
    mutationFn: (action: SwitchAction) => api.settings.setSwitch(action),
    onSuccess: (next) => client.setQueryData(queryKeys.switches, next),
  });
  const busy = act.isPending ? act.variables.name : null;
  const toggle = (key: string) => setOpen(open === key ? null : key);
  if (page.isPending) return <ListRow title="Looking at Emberhut's servers…" />;
  if (page.isError) return <ListRow title={`Can't read Emberhut's servers: ${page.error.message}`} destructive />;
  return (
    <>
      <Section
        title="MCP servers"
        count={page.data.mcp.length}
        footer={
          page.data.note ??
          `What Emberhut's own session has${page.data.account ? ` (on ${page.data.account})` : ""}. Off: Emberhut's sessions start again without it once their turn is over; the runners' at their next start.`
        }
      >
        {page.data.mcp.map((server) => (
          <McpRow
            key={server.name}
            server={server}
            open={open === `mcp:${server.name}`}
            busy={busy}
            onOpen={() => toggle(`mcp:${server.name}`)}
            onAction={(action) => act.mutate(action)}
          />
        ))}
      </Section>
      <Section
        title="Skills"
        count={page.data.skills.length}
        footer="Off: Emberhut, its workers and its runner sessions don't see it. Your own sessions keep it."
      >
        {page.data.skills.map((skill) => (
          <SkillRow
            key={skill.name}
            skill={skill}
            open={open === `skill:${skill.name}`}
            busy={busy}
            onOpen={() => toggle(`skill:${skill.name}`)}
            onAction={(a) => act.mutate(a)}
          />
        ))}
      </Section>
      {act.isError ? (
        <Text variant="footnote" tone="danger">
          {act.error.message}
        </Text>
      ) : null}
    </>
  );
}

function McpRow({
  server,
  open,
  busy,
  onOpen,
  onAction,
}: {
  server: Mcp;
  open: boolean;
  busy: string | null;
  onOpen: () => void;
  onAction: (a: SwitchAction) => void;
}) {
  const { colors, spacing } = useTheme();
  const status = server.state === "failed" && server.error ? `Failed: ${server.error}` : mcpStatus[server.state];
  return (
    <View style={{ backgroundColor: colors.surface }}>
      <View
        style={{ flexDirection: "row", alignItems: "center", gap: spacing.md, paddingHorizontal: spacing.lg, paddingVertical: spacing.sm }}
      >
        <Pressable
          onPress={onOpen}
          accessibilityRole="button"
          accessibilityLabel={`${server.name}, ${status}`}
          accessibilityState={{ expanded: open }}
          style={{ flex: 1, flexDirection: "row", alignItems: "center", gap: spacing.md, minHeight: 40 }}
        >
          <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: colors[mcpColor[server.state]] }} />
          <View style={{ flex: 1 }}>
            <Text variant="callout" numberOfLines={1}>
              {server.name}
            </Text>
            <Text variant="footnote" tone={server.state === "failed" ? "danger" : "secondary"} numberOfLines={open ? undefined : 1}>
              {status}
            </Text>
          </View>
        </Pressable>
        {server.fixed ? (
          <Icon name="lock-closed" size={16} color={colors.textTertiary} accessibilityLabel="Emberhut's own tools: always on" />
        ) : (
          <Toggle
            value={server.on}
            onChange={(on) => onAction({ kind: "mcp", name: server.name, on })}
            label={server.name}
            disabled={busy !== null}
          />
        )}
      </View>
      {open ? (
        <View style={{ paddingHorizontal: spacing.lg, paddingLeft: spacing.lg + 20, paddingBottom: spacing.md, gap: spacing.xs }}>
          {server.note ? (
            <Text variant="footnote" tone="secondary">
              {server.note}
            </Text>
          ) : null}
          <Text variant="caption" tone="tertiary">
            {[server.kind, server.source === "user" ? "your Claude config" : server.source].filter(Boolean).join(" · ")}
          </Text>
          {server.on && !server.fixed && server.state !== "connected" ? (
            <View style={{ alignSelf: "flex-start" }}>
              <Button
                label="Reconnect"
                icon="refresh"
                variant="plain"
                onPress={() => onAction({ kind: "reconnect", name: server.name })}
                loading={busy === server.name}
              />
            </View>
          ) : null}
        </View>
      ) : null}
    </View>
  );
}

function SkillRow({
  skill,
  open,
  busy,
  onOpen,
  onAction,
}: {
  skill: Skill;
  open: boolean;
  busy: string | null;
  onOpen: () => void;
  onAction: (a: SwitchAction) => void;
}) {
  const { colors, spacing } = useTheme();
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        paddingHorizontal: spacing.lg,
        paddingVertical: spacing.sm,
        backgroundColor: colors.surface,
      }}
    >
      <Pressable
        onPress={onOpen}
        accessibilityRole="button"
        accessibilityLabel={skill.name}
        accessibilityState={{ expanded: open }}
        style={{ flex: 1, minHeight: 40, justifyContent: "center" }}
      >
        <Text variant="callout" tone={skill.on ? "primary" : "secondary"} numberOfLines={1}>
          {skill.name}
          {skill.source !== "yours" ? <Text variant="caption" tone="tertiary">{`  ${skill.source}`}</Text> : null}
        </Text>
        {skill.description ? (
          <Text variant="footnote" tone="secondary" numberOfLines={open ? undefined : 1}>
            {skill.description}
          </Text>
        ) : null}
      </Pressable>
      <Toggle
        value={skill.on}
        onChange={(on) => onAction({ kind: "skill", name: skill.name, on })}
        label={skill.name}
        disabled={busy !== null}
      />
    </View>
  );
}
