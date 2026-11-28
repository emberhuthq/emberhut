import { useLayoutEffect, useState, type ReactNode } from "react";
import { Alert, View } from "react-native";
import type { Fact, SettingsChange } from "@/api";
import type { RootScreenProps } from "@/navigation/types";
import { useFacts } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, EmptyState, ListRow, Loading, Screen, Section, Text, TextField, Toggle } from "@/ui";
import { forgetEverywhere, whyIKnow } from "./facts";
import { useFactChange } from "./useFactChange";

/** Fix a fact (your ember uses your version from then on), wake your ember for a person's messages, or forget it. */
export function FactScreen({ route, navigation }: RootScreenProps<"Fact">) {
  const facts = useFacts();
  // Held here, not in the editor: a forget hides the fact at once, and its failure must still show when it comes back.
  const change = useFactChange();
  const fact = facts.data?.facts.find((f) => f.id === route.params.id);
  const close = () => navigation.goBack();
  const send = (body: SettingsChange) => change.mutate(body, { onSuccess: close });
  const failed = change.isError ? (
    <Text variant="footnote" tone="danger">
      Not saved: {change.error.message}
    </Text>
  ) : null;
  if (facts.isPending) return <Loading />;
  if (!fact) {
    if (change.isPending) return <Loading />;
    return <EmptyState icon="person-circle-outline" title="Forgotten" message="Your ember doesn't keep this one anymore." />;
  }
  return (
    <FactEditor fact={fact} send={send} sending={change.isPending} failed={failed} setTitle={(title) => navigation.setOptions({ title })} />
  );
}

interface EditorProps {
  fact: Fact;
  /** Sends a change; the page closes once the server takes a fix or a forget. */
  send: (change: SettingsChange) => void;
  sending: boolean;
  failed: ReactNode;
  setTitle: (title: string) => void;
}

function FactEditor({ fact, send, sending, failed, setTitle }: EditorProps) {
  const { spacing } = useTheme();
  const [text, setText] = useState(fact.text);
  const change = useFactChange();
  useLayoutEffect(() => setTitle(fact.title), [fact.title, setTitle]);
  const unchanged = text.trim() === "" || text.trim() === fact.text.trim();
  const save = () => send({ factSet: { id: fact.id, category: fact.category, title: fact.title, text: text.trim() } });
  const forget = () =>
    Alert.alert(`Forget "${fact.title}" everywhere?`, forgetEverywhere, [
      { text: "Cancel", style: "cancel" },
      {
        text: "Forget",
        style: "destructive",
        onPress: () => send({ factDrop: fact.id }),
      },
    ]);
  return (
    <Screen scroll>
      <View style={{ gap: spacing.sm }}>
        <TextField
          value={text}
          onChangeText={setText}
          multiline
          accessibilityLabel={`What's true about ${fact.title}`}
          placeholder="What's true"
        />
        <Text variant="footnote" tone="secondary" style={{ paddingHorizontal: spacing.xs }}>
          {whyIKnow(fact) ?? `From ${fact.source || "your ember"}`}. Your ember will use your version from now on.
        </Text>
        <Button label="Save" onPress={save} disabled={unchanged || sending} />
        {failed}
      </View>
      {fact.category === "people" ? (
        <Section
          title="What's going on now"
          footer="Your ember keeps this note from your chats. Important people's messages reach it at once instead of every 15 minutes."
        >
          {fact.now ? (
            <View key="now" style={{ padding: spacing.lg }}>
              <Text selectable>{fact.now}</Text>
            </View>
          ) : null}
          <ListRow
            key="wake"
            title="Wake your ember when they write"
            trailing={
              <Toggle
                value={fact.important === true}
                onChange={(important) => change.mutate({ personSet: { id: fact.id, important } })}
                label="Wake your ember when they write"
              />
            }
          />
        </Section>
      ) : null}
      <Section
        title="What your ember always knows"
        footer="A few lines about you are in front of your ember every time it starts. Pin this to keep it there, or leave it out."
      >
        {fact.pending ? (
          <ListRow
            key="confirm"
            title="Waiting for you to confirm it"
            trailing={<Button label="Confirm" onPress={() => change.mutate({ factConfirm: fact.id })} disabled={change.isPending} />}
          />
        ) : null}
        <ListRow
          key="pin"
          title="Always keep this in mind"
          trailing={
            <Toggle
              value={fact.profile === "pin"}
              onChange={(on) => change.mutate({ factProfile: { id: fact.id, profile: on ? "pin" : null } })}
              label="Always keep this in mind"
            />
          }
        />
        <ListRow
          key="hide"
          title="Leave it out"
          trailing={
            <Toggle
              value={fact.profile === "hide"}
              onChange={(on) => change.mutate({ factProfile: { id: fact.id, profile: on ? "hide" : null } })}
              label="Leave it out"
            />
          }
        />
      </Section>
      {change.isError ? (
        <Text variant="footnote" tone="danger">
          Not saved: {change.error.message}
        </Text>
      ) : null}
      <Button label="Forget this everywhere" variant="destructive" icon="trash-outline" onPress={forget} disabled={sending} />
    </Screen>
  );
}
