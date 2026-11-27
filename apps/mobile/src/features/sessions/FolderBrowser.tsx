import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ScrollView } from "react-native";
import { api } from "@/services";
import { queryKeys } from "@/state/query";
import { useTheme } from "@/theme";
import { Button, ListRow, Loading, Screen, Section, Text, TextField } from "@/ui";
import { shortPath } from "./sessionList";

/** A folder on a machine: browse in from Documents, or type its path. `place` is the machine's name from the server. */
export function FolderBrowser({
  machine,
  place,
  start,
  onPick,
  onCancel,
}: {
  machine: string;
  place: string;
  start: string;
  onPick: (path: string) => void;
  onCancel: () => void;
}) {
  const { spacing } = useTheme();
  const [trail, setTrail] = useState<string[]>([start]);
  const [typed, setTyped] = useState("");
  const path = trail.at(-1)!;
  const listing = useQuery({ queryKey: queryKeys.folders(machine, path), queryFn: () => api.sessions.folders(machine, path) });
  return (
    <Screen padded={false} edges={["bottom"]}>
      <ScrollView contentContainerStyle={{ padding: spacing.lg, gap: spacing.lg }} keyboardShouldPersistTaps="handled">
        <Section title={`On ${place}`} footer={shortPath(listing.data?.path ?? path)}>
          <ListRow icon="checkmark-circle" title="Use this folder" onPress={() => onPick(listing.data?.path ?? path)} opens={false} />
          {trail.length > 1 && <ListRow icon="arrow-back" title="Up" onPress={() => setTrail((t) => t.slice(0, -1))} opens={false} />}
        </Section>
        {listing.isPending ? (
          <Loading />
        ) : listing.isError ? (
          <Text tone="secondary">{`The runner on ${place} didn't answer.`}</Text>
        ) : listing.data.folders.length > 0 ? (
          <Section title="Inside">
            {listing.data.folders.map((f) => (
              <ListRow
                key={f.path}
                icon={f.git ? "git-branch-outline" : "folder-outline"}
                title={f.name}
                onPress={() => setTrail((t) => [...t, f.path])}
              />
            ))}
          </Section>
        ) : null}
        <Section title="Or type it">
          <TextField
            value={typed}
            onChangeText={setTyped}
            placeholder="A folder's path"
            autoCapitalize="none"
            autoCorrect={false}
            onSubmitEditing={() => typed.trim() && onPick(typed.trim())}
            accessibilityLabel="A folder's path"
          />
        </Section>
        <Button label="Cancel" variant="plain" onPress={onCancel} />
      </ScrollView>
    </Screen>
  );
}
