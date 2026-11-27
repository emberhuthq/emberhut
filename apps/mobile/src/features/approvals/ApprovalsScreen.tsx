import { useIsFocused } from "@react-navigation/native";
import { useEffect, useLayoutEffect, useMemo } from "react";
import { View } from "react-native";
import { isOlderServer } from "@/features/machines/machines";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { useAgents, useApprovals, useEmberhutState } from "@/state/queries";
import { useTheme } from "@/theme";
import { EmptyState, ErrorState, Loading, Screen, SectionHeading, Text, usePullRefresh } from "@/ui";
import { ApprovalCard } from "./ApprovalParts";
import { approvalGroups, visible } from "./model";

const olderServerText = "Emberhut's server is on older code: approvals come with its next update";

/**
 * Pending approvals: what waits on you (open one to approve or reject its exact payload), then what waits on an approver,
 * shown for information. With `agent`, only that agent's (from its room's header).
 */
export function ApprovalsScreen({ route, navigation }: RootScreenProps<"Approvals">) {
  const { spacing } = useTheme();
  const agent = route.params?.agent;
  const focused = useIsFocused();
  const page = useApprovals({ state: "pending", agent }, focused);
  const pull = usePullRefresh(page.refetch);
  const state = useEmberhutState();
  const agents = useMemo(() => state.data?.agents ?? [], [state.data?.agents]);
  const settings = useAgents();
  const currency = useMemo(() => new Map((settings.data ?? []).map((a) => [a.id, a.spendCurrency ?? null])), [settings.data]);
  const name = agent === undefined ? undefined : visible(agents.find((a) => a.id === agent)?.name ?? agent);
  const { refetch } = page;

  useLayoutEffect(() => {
    navigation.setOptions({ title: name === undefined ? "Approvals" : `${name}: approvals` });
  }, [navigation, name]);
  // Back from a request, what was decided there is gone from the list at once.
  useEffect(() => {
    if (focused) inBackground(refetch(), "ApprovalsScreen: refetch");
  }, [focused, refetch]);

  if (page.isPending) return <Loading label="Looking…" />;
  if (!page.data) {
    if (isOlderServer(page.error)) return <EmptyState icon="shield-checkmark-outline" title="Not yet" message={olderServerText} />;
    return (
      <ErrorState
        message={page.error?.message ?? "Couldn't read the approvals"}
        onRetry={() => inBackground(refetch(), "ApprovalsScreen: refetch")}
      />
    );
  }
  const { yours, others } = approvalGroups(page.data);
  const open = (id: number) => navigation.navigate("Approval", { id });
  if (yours.length === 0 && others.length === 0) {
    return (
      <Screen scroll {...pull}>
        <EmptyState
          icon="shield-checkmark-outline"
          title="Nothing waits for an OK"
          message={
            name === undefined
              ? "When an agent asks before it posts, sends, merges, deploys or pays, the request shows here with exactly what it would do."
              : `When ${name} asks before it acts, the request shows here with exactly what it would do.`
          }
        />
      </Screen>
    );
  }
  return (
    <Screen scroll {...pull}>
      {yours.length > 0 && (
        <View style={{ gap: spacing.sm }}>
          <SectionHeading tone="needsYou">Waiting on you · {yours.length}</SectionHeading>
          {yours.map((approval) => (
            <ApprovalCard
              key={approval.id}
              approval={approval}
              agents={agents}
              currency={approval.amountCurrency ?? currency.get(approval.agent)}
              onPress={() => open(approval.id)}
            />
          ))}
        </View>
      )}
      {others.length > 0 && (
        <View style={{ gap: spacing.sm }}>
          <SectionHeading>Waiting on others · {others.length}</SectionHeading>
          {others.map((approval) => (
            <ApprovalCard
              key={approval.id}
              approval={approval}
              agents={agents}
              currency={approval.amountCurrency ?? currency.get(approval.agent)}
              onPress={() => open(approval.id)}
            />
          ))}
          <Text variant="footnote" tone="secondary" style={{ paddingHorizontal: spacing.xs }}>
            Their approver decides these, from Slack, a link or their own app. You see them so you know what is asked.
          </Text>
        </View>
      )}
    </Screen>
  );
}
