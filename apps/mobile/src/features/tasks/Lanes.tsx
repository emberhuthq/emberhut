import { useState } from "react";
import { View } from "react-native";
import type { AreaTree } from "@/api";
import { useTheme } from "@/theme";
import { FoldRow, ListRow, Section, Text } from "@/ui";
import { areaProgress, splitDone, type Lane } from "./laneModel";
import { AreaStateMark } from "./StateMark";
import { WorkItemRow } from "./WorkItemRow";

/** A lane of a project's work: its area's state and counts, its open work with the steps nested, done folded under it. */
export function LaneSection({ lane }: { lane: Lane }) {
  const { spacing } = useTheme();
  const [showDone, setShowDone] = useState(false);
  const { open, done } = splitDone(lane.items);
  return (
    <View style={{ gap: spacing.xs }}>
      {lane.area ? <AreaHead area={lane.area} /> : null}
      <Section title={lane.area ? undefined : lane.title}>
        {open.map((item) => (
          <WorkItemRow key={item.id} item={item} />
        ))}
        {done.length > 0 ? (
          <FoldRow key="done" title={`${done.length} done`} open={showDone} onToggle={() => setShowDone(!showDone)} />
        ) : null}
        {showDone ? done.map((item) => <WorkItemRow key={item.id} item={item} />) : null}
        {lane.area && lane.area.findings.total > 0 ? <FindingsRow key="findings" area={lane.area} /> : null}
        {lane.items.length === 0 ? <ListRow key="empty" title="No tasks yet" /> : null}
      </Section>
    </View>
  );
}

function AreaHead({ area }: { area: AreaTree }) {
  const { spacing } = useTheme();
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm, paddingHorizontal: spacing.xs }}>
      <AreaStateMark state={area.state} size={16} />
      <View style={{ flex: 1 }}>
        <Text variant="headline" accessibilityRole="header">
          {area.name}
        </Text>
        <Text variant="footnote" tone="secondary">
          {areaProgress(area.counts)}
        </Text>
      </View>
    </View>
  );
}

/** An area's findings: how many landed and how many didn't, in a row. */
function FindingsRow({ area }: { area: AreaTree }) {
  const f = area.findings;
  const open = f.partly + f.notLanded + f.unchecked;
  const parts = [
    f.notLanded > 0 ? `${f.notLanded} not landed` : null,
    f.partly > 0 ? `${f.partly} partly` : null,
    f.unchecked > 0 ? `${f.unchecked} not verified` : null,
    f.landed > 0 ? `${f.landed} landed` : null,
  ].filter(Boolean);
  return <ListRow icon="search-outline" title={`Findings · ${open} open of ${f.total}`} subtitle={parts.join(" · ")} />;
}
