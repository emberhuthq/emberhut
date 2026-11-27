import { useLayoutEffect, useState, type ReactNode } from "react";
import { Alert, View } from "react-native";
import type { Fact, SettingsChange } from "@/api";
import type { RootScreenProps } from "@/navigation/types";
import { useFacts } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, EmptyState, ListRow, Loading, Screen, Section, Text, TextField, Toggle } from "@/ui";
import { useFactChange } from "./useFactChange";

/** Fix a fact (Emberhut uses your version from then on), wake Emberhut for a person's messages, or forget it. */
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
    return <EmptyState icon="person-circle-outline" title="Forgotten" message="Emberhut doesn't keep this one anymore." />;
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
    Alert.alert(`Forget "${fact.title}"?`, "Emberhut stops using it.", [
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
          From {fact.source || "Emberhut"}. Emberhut will use your version from now on.
        </Text>
        <Button label="Save" onPress={save} disabled={unchanged || sending} />
        {failed}
      </View>
      {fact.category === "people" ? (
        <Section
          title="What's going on now"
          footer="Emberhut keeps this note from your chats. Important people's messages reach Emberhut at once instead of every 15 minutes."
        >
          {fact.now ? (
            <View key="now" style={{ padding: spacing.lg }}>
              <Text selectable>{fact.now}</Text>
            </View>
          ) : null}
          <ListRow
            key="wake"
            title="Wake Emberhut when they write"
            trailing={
              <Toggle
                value={fact.important === true}
                onChange={(important) => change.mutate({ personSet: { id: fact.id, important } })}
                label="Wake Emberhut when they write"
              />
            }
          />
        </Section>
      ) : null}
      {change.isError ? (
        <Text variant="footnote" tone="danger">
          Not saved: {change.error.message}
        </Text>
      ) : null}
      <Button label="Forget this" variant="destructive" icon="trash-outline" onPress={forget} disabled={sending} />
    </Screen>
  );
}
