import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { View } from "react-native";
import type { Approval } from "@/api";
import { approvalGroups, approvalsSummary } from "@/features/approvals/model";
import { inBackground } from "@/log";
import { useActions, useConnection, usePrefs } from "@/state/hooks";
import { offlineReason } from "@/state/connection";
import type { NeedsGroupingPreference } from "@/state/prefs";
import { useEmberhutState } from "@/state/queries";
import { useTheme } from "@/theme";
import { Chip, EmptyState, ErrorState, FoldRow, ListRow, Loading, Screen, Section, SectionHeading, Segmented, usePullRefresh } from "@/ui";
import { byUrgency, needsGroups, needsRows, projectGroups, whenNames } from "./model";
import { NeedsCard } from "./NeedsCard";

const groupings: readonly { value: NeedsGroupingPreference; label: string }[] = [
  { value: "project", label: "By project" },
  { value: "when", label: "By time" },
];

/**
 * Only what waits on you, grouped by project: a section
 * per project, the one with the most urgent ask first, Other (no project) last; in each, the most urgent first, each card
 * tagged Today, This week or This month (overdue in red). The other grouping (kept): by when you have to decide, Today (with the
 * overdue ones), This week, This month, the cheap ones first. Either way Later is folded and counted nowhere, so it doesn't
 * pull at you now, and the sections add up to the tab's badge. Approval requests come first, as one row with how many
 * wait on you (counted in the badge too) and on others, opening the Approvals list: each is an agent's exact payload to
 * read whole before deciding, which a card here couldn't hold.
 */
export function NeedsYouScreen() {
  const { spacing } = useTheme();
  const state = useEmberhutState();
  const pull = usePullRefresh(state.refetch);
  const gone = useActions((s) => s.gone);
  const offline = useConnection(offlineReason);
  const grouping = usePrefs((s) => s.needsGrouping);
  const setGrouping = usePrefs((s) => s.setNeedsGrouping);
  const [laterOpen, setLaterOpen] = useState(false);
  const navigation = useNavigation();
  if (!state.data) {
    return (
      <Screen>
        {state.isError && offline ? (
          <ErrorState message={offline} onRetry={() => inBackground(state.refetch(), "NeedsYouScreen: refetch")} />
        ) : (
          <Loading label="Reaching Emberhut" />
        )}
      </Screen>
    );
  }
  const issues = state.data.issues ?? [];
  const byProject = grouping !== "when";
  const hidden = state.data.ui?.hideProjects === true;
  // Grouped by project, Later's cards name theirs even with the projects screens hidden: it's what the page is grouped by.
  const project = (pid: number | null | undefined) => (hidden && !byProject ? null : state.data.projects?.find((p) => p.id === pid)?.name);
  const rows = needsRows(state.data, gone);
  const groups = needsGroups(rows);
  const now = groups.filter((g) => g.when !== "later");
  const folded = groups.find((g) => g.when === "later")?.rows ?? [];
  const later = byProject ? byUrgency(folded) : folded;
  const approvals = state.data.approvals ?? [];
  const waiting = approvalGroups(approvals).yours.length;
  const approvalsRow =
    approvals.length > 0 ? <ApprovalsRow approvals={approvals} onPress={() => navigation.navigate("Approvals")} /> : null;
  if (groups.length === 0) {
    return (
      <Screen scroll {...pull}>
        {approvalsRow}
        {waiting > 0 ? null : (
          <EmptyState
            icon="checkmark-circle-outline"
            title="Nothing needs you"
            message="When Emberhut needs a decision, or something only you can do, it shows here."
          />
        )}
      </Screen>
    );
  }
  return (
    <Screen scroll {...pull}>
      {approvalsRow}
      <Segmented options={groupings} value={byProject ? "project" : "when"} onChange={setGrouping} accessibilityLabel="Group Needs you" />
      {byProject
        ? projectGroups(rows, state.data.projects ?? []).map((group) => (
            <View key={group.pid ?? "other"} style={{ gap: spacing.sm }}>
              <SectionHeading tone="needsYou">
                {group.name} · {group.rows.length}
              </SectionHeading>
              {group.rows.map((row) => (
                <NeedsCard key={row.id} row={row} issues={issues} tier />
              ))}
            </View>
          ))
        : now.map((group) => (
            <View key={group.when} style={{ gap: spacing.sm }}>
              <SectionHeading tone="needsYou">
                {whenNames[group.when]} · {group.rows.length}
              </SectionHeading>
              {group.rows.map((row) => (
                <NeedsCard key={row.id} row={row} issues={issues} project={project(row.pid)} />
              ))}
            </View>
          ))}
      {later.length > 0 ? (
        <View style={{ gap: spacing.sm }}>
          <Section
            footer={
              hidden
                ? "More than a month out: not counted, so it waits quietly."
                : "More than a month out, or in a paused project: not counted, so it waits quietly."
            }
          >
            <FoldRow title={`Later · ${later.length}`} open={laterOpen} onToggle={() => setLaterOpen(!laterOpen)} />
          </Section>
          {laterOpen ? later.map((row) => <NeedsCard key={row.id} row={row} issues={issues} project={project(row.pid)} />) : null}
        </View>
      ) : null}
    </Screen>
  );
}

/** The pending approvals as one row: how many wait on you, how many of those only you can approve, and those on others. */
function ApprovalsRow({ approvals, onPress }: { approvals: readonly Approval[]; onPress: () => void }) {
  const summary = approvalsSummary(approvals);
  return (
    <Section title="Approvals" tone={summary.yours > 0 ? "needsYou" : "secondary"}>
      <ListRow
        key="approvals"
        icon="shield-checkmark-outline"
        title={summary.title}
        subtitle={summary.subtitle ?? undefined}
        trailing={summary.yours > 0 ? <Chip label={String(summary.yours)} tone="needsYou" strong /> : undefined}
        onPress={onPress}
      />
    </Section>
  );
}
