import type { ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import type { AgentRow, Approval } from "@/api";
import { parseInstant, relative } from "@/format/time";
import { monoFont, useTheme } from "@/theme";
import { Card, Chip, Text } from "@/ui";
import {
  agentName,
  amountText,
  approvalCardLabel,
  approverRole,
  fileLine,
  isFixedCore,
  kindName,
  shortHash,
  stateName,
  visible,
  waitsOnYou,
} from "./model";

/** What it is for, and the fixed core's mark: only you can approve a payment, a secret, a password, account security or anything in your name. */
export function KindChips({ approval }: { approval: Approval }) {
  const { spacing } = useTheme();
  const core = isFixedCore(approval.kind);
  return (
    <View style={[styles.row, { gap: spacing.xs }]}>
      <Chip label={kindName(approval.kind)} tone={core ? "danger" : "needsYou"} />
      {core && <Chip label="Only you can approve this" tone="danger" strong icon="lock-closed" />}
      {!waitsOnYou(approval) && <Chip label={stateName(approval.state)} tone="muted" />}
    </View>
  );
}

/** A request in the list: its agent, kind, one line and where it goes; who decides it when that isn't you. A tap opens it whole. */
export function ApprovalCard({
  approval,
  agents,
  currency,
  onPress,
}: {
  approval: Approval;
  agents: readonly AgentRow[];
  /** A payment's currency: its agent's. */
  currency?: string | null;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  const files = approval.files?.length ?? 0;
  const yours = waitsOnYou(approval);
  const asked = parseInstant(approval.createdAt);
  const facts = [
    agentName(approval, agents),
    files > 0 ? `${files} file${files === 1 ? "" : "s"}` : null,
    asked ? relative(asked) : null,
  ].filter(Boolean);
  return (
    <Card
      onPress={onPress}
      accent={yours ? (isFixedCore(approval.kind) ? colors.danger : colors.needsYou) : undefined}
      accessibilityLabel={approvalCardLabel(approval, agents, currency)}
    >
      <KindChips approval={approval} />
      <Text weight="600" numberOfLines={3}>
        {visible(approval.summary)}
      </Text>
      <Text variant="footnote" tone="secondary" numberOfLines={2}>
        To {visible(approval.target)}
      </Text>
      {approval.amountCents != null && <Text weight="600">{amountText(approval.amountCents, currency)}</Text>}
      <Text variant="caption" tone="tertiary">
        {facts.join(" · ")}
      </Text>
      {!yours && (
        <Text variant="footnote" tone="secondary">
          Waits on {approverRole(approval)}
        </Text>
      )}
    </Card>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <View style={styles.field}>
      <Text variant="footnote" tone="secondary" weight="600">
        {label}
      </Text>
      {children}
    </View>
  );
}

/**
 * The exact payload a decision covers, nothing summarised away: the agent, the kind, the target, the text word for word
 * (fixed width, selectable), each file's name, size and the start of its SHA-256, a payment's amount, and the payload's
 * own hash, which the decision sends back.
 */
export function Payload({ approval, agent, currency }: { approval: Approval; agent: AgentRow | undefined; currency?: string | null }) {
  const { colors, radius, spacing } = useTheme();
  const files = approval.files ?? [];
  const asked = parseInstant(approval.createdAt);
  const expires = parseInstant(approval.expiresAt);
  const name = agentName(approval, agent ? [agent] : []);
  return (
    <Card accent={isFixedCore(approval.kind) && waitsOnYou(approval) ? colors.danger : undefined}>
      <KindChips approval={approval} />
      <Text variant="headline" selectable>
        {visible(approval.summary)}
      </Text>
      <Field label="Agent">
        <Text selectable>
          {name}
          {agent ? ` · works for ${visible(agent.principal)}` : ""}
        </Text>
      </Field>
      <Field label="To">
        <Text selectable>{visible(approval.target)}</Text>
      </Field>
      {approval.amountCents != null && (
        <Field label="Amount">
          <Text variant="title" selectable>
            {amountText(approval.amountCents, currency)}
          </Text>
        </Field>
      )}
      {approval.text != null && (
        <Field label="Exact text">
          <View style={{ backgroundColor: colors.surfaceRaised, borderRadius: radius.sm, padding: spacing.md }}>
            <Text variant="callout" selectable style={styles.mono} accessibilityLabel={`Exact text: ${visible(approval.text)}`}>
              {visible(approval.text)}
            </Text>
          </View>
        </Field>
      )}
      {files.length > 0 && (
        <Field label={files.length === 1 ? "File" : `Files · ${files.length}`}>
          {files.map((file) => (
            <Text key={`${file.sha256}:${file.name}`} variant="footnote" selectable style={styles.mono}>
              {fileLine(file)}
            </Text>
          ))}
        </Field>
      )}
      <Field label="Decides">
        <Text>{approverRole(approval)}</Text>
      </Field>
      <Text variant="caption" tone="tertiary" selectable>
        {[
          asked ? `Asked ${relative(asked)}` : null,
          expires && approval.state === "pending" ? `expires ${relative(expires)}` : null,
          `payload ${shortHash(approval.payloadHash)}`,
        ]
          .filter(Boolean)
          .join(" · ")}
      </Text>
    </Card>
  );
}

/** How it ended, or where it stands, for a request that's no longer yours to decide. */
export function Outcome({ approval }: { approval: Approval }) {
  const decided = parseInstant(approval.decidedAt);
  const used = parseInstant(approval.usedAt);
  const lines = [
    `${stateName(approval.state)}${approval.stateReason ? `: ${visible(approval.stateReason)}` : ""}`,
    decided
      ? `Decided ${relative(decided)}${approval.decidedBy ? ` by ${visible(approval.decidedBy)}` : ""}${approval.decidedVia ? ` (${approval.decidedVia})` : ""}`
      : null,
    used ? `Used ${relative(used)}${approval.usedBy ? ` by ${visible(approval.usedBy)}` : ""}` : null,
    approval.imported ? "From an earlier installation's history: it can't be used." : null,
  ].filter(Boolean);
  return (
    <Text variant="footnote" tone="secondary" selectable>
      {lines.join("\n")}
    </Text>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", flexWrap: "wrap", alignItems: "center" },
  field: { gap: 2 },
  mono: { fontFamily: monoFont },
});
