import * as Haptics from "expo-haptics";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Alert, View } from "react-native";
import { taskKey, type Approval } from "@/api";
import { isOlderServer } from "@/features/machines/machines";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { useAgents, useApproval, useEmberhutState } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, EmptyState, ErrorState, ListRow, Loading, Screen, Section, Text, usePullRefresh } from "@/ui";
import { Outcome, Payload } from "./ApprovalParts";
import { canWithdraw, decideOn, stillAsked } from "./decision";
import { approveQuestion, decisionFailure, kindName, waitsOnYou } from "./model";
import { useDecision } from "./useDecision";

/**
 * One request, fetched as it is now, with its whole payload. When it waits on you: Approve (after a confirmation) or
 * Reject, each sending the payload hash of exactly what is on screen, so a request that changed meanwhile decides nothing
 * (409) and is shown again as it is. One answer goes at a time, and a confirmation that outlives a re-read of the request
 * decides nothing. A pending request, or an approved one not used yet, can be withdrawn. Anything else is read-only.
 */
export function ApprovalScreen({ route, navigation }: RootScreenProps<"Approval">) {
  const { id } = route.params;
  // A link from outside the app may carry anything: only a request's number is asked for.
  if (!Number.isInteger(id) || id <= 0) {
    return <EmptyState icon="shield-checkmark-outline" title="Gone" message="This request isn't there anymore." />;
  }
  return <ApprovalPage id={id} navigation={navigation} />;
}

function ApprovalPage({ id, navigation }: { id: number; navigation: RootScreenProps<"Approval">["navigation"] }) {
  const { spacing } = useTheme();
  const query = useApproval(id);
  const pull = usePullRefresh(query.refetch);
  const state = useEmberhutState();
  const agents = state.data?.agents ?? [];
  const settings = useAgents();
  const decision = useDecision(id);
  const shown = query.data;
  // The request as it is now, for a question that outlives a re-read.
  const latest = useRef(shown);
  useEffect(() => {
    latest.current = shown;
  }, [shown]);
  const [changed, setChanged] = useState(false);

  useLayoutEffect(() => {
    if (shown) navigation.setOptions({ title: kindName(shown.kind) });
  }, [navigation, shown]);

  if (query.isPending) return <Loading label="Looking…" />;
  if (!shown) {
    if (isOlderServer(query.error))
      return <EmptyState icon="shield-checkmark-outline" title="Gone" message="This request isn't there anymore." />;
    return (
      <ErrorState
        message={query.error?.message ?? "Couldn't read this request"}
        onRetry={() => inBackground(query.refetch(), "ApprovalScreen: refetch")}
      />
    );
  }

  const agent = agents.find((a) => a.id === shown.agent);
  const currency = shown.amountCurrency ?? settings.data?.find((a) => a.id === shown.agent)?.spendCurrency ?? null;
  const yours = waitsOnYou(shown);
  const busy = decision.isPending;
  const { turn } = decision;
  // Each action carries the request it was taken on: the hash sent is the one of the payload on screen. A question holds
  // the turn, so a second tap opens no second one.
  const approve = (approval: Approval) => {
    if (!turn.take()) return;
    setChanged(false);
    const { title, message } = approveQuestion(approval, agents, currency);
    Alert.alert(
      title,
      message,
      [
        { text: "Cancel", style: "cancel", onPress: () => turn.release() },
        {
          text: "Approve",
          onPress: () => {
            if (!stillAsked(approval, latest.current)) {
              turn.release();
              setChanged(true);
              return;
            }
            inBackground(Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success), "ApprovalScreen: haptics");
            decision.send(decideOn(approval, "approve"), true);
          },
        },
      ],
      { cancelable: true, onDismiss: () => turn.release() },
    );
  };
  const reject = (approval: Approval) => {
    setChanged(false);
    decision.send(decideOn(approval, "reject"));
  };
  const withdraw = () => {
    if (!turn.take()) return;
    Alert.alert(
      "Withdraw this request?",
      "Nothing it covers goes out. The agent has to ask again.",
      [
        { text: "Cancel", style: "cancel", onPress: () => turn.release() },
        { text: "Withdraw", style: "destructive", onPress: () => decision.send({ kind: "withdraw" }, true) },
      ],
      { cancelable: true, onDismiss: () => turn.release() },
    );
  };

  return (
    <Screen scroll {...pull}>
      {decision.isError && (
        <Text tone="danger" accessibilityRole="alert">
          {decisionFailure(decision.error).message}
        </Text>
      )}
      {changed && (
        <Text tone="danger" accessibilityRole="alert">
          This request changed while you were asked, so nothing was done. Here it is as it is now.
        </Text>
      )}
      <Payload approval={shown} agent={agent} currency={currency} />
      {shown.state !== "pending" && <Outcome approval={shown} />}
      {yours ? (
        <View style={{ gap: spacing.sm }}>
          <Button label="Approve" icon="checkmark" onPress={() => approve(shown)} loading={busy} block />
          <Button label="Reject" icon="close" variant="destructive" onPress={() => reject(shown)} disabled={busy} block />
          <Text variant="footnote" tone="secondary" style={{ paddingHorizontal: spacing.xs }}>
            Approving covers exactly what is shown here, once. A changed word, file or target needs a new OK.
          </Text>
        </View>
      ) : shown.state === "pending" ? (
        <Text variant="footnote" tone="secondary" style={{ paddingHorizontal: spacing.xs }}>
          This waits on its approver, not on you: you see it so you know what is asked.
        </Text>
      ) : null}
      {shown.task != null && (
        <Section>
          <ListRow
            title={`From ${taskKey(shown.task)}`}
            icon="list-outline"
            onPress={() => shown.task != null && navigation.navigate("Task", { id: shown.task })}
          />
        </Section>
      )}
      {canWithdraw(shown) && (
        <Button label="Withdraw request" variant="plain" onPress={withdraw} disabled={busy} accessibilityLabel="Withdraw this request" />
      )}
    </Screen>
  );
}
