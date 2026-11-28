import { useState } from "react";
import { View } from "react-native";
import type { RootScreenProps } from "@/navigation/types";
import { useEmberhutState, useRenameTask } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, Screen, Text, TextField } from "@/ui";
import { newTitle, renameFailure } from "./rename";

/**
 * Rename, from a task page's menu: the title in a field, Save (or the keyboard's Done) sends it, trimmed; the same title
 * just closes. Once the server takes it the page and the lists say the new title; a refusal (an empty or too long title,
 * an unknown task) shows the server's reason and the field stays for another try.
 */
export function RenameTaskScreen({ route, navigation }: RootScreenProps<"RenameTask">) {
  const { id } = route.params;
  const { spacing } = useTheme();
  const state = useEmberhutState();
  const current = state.data?.issues?.find((t) => t.id === id)?.title ?? route.params.title;
  const [typed, setTyped] = useState(current);
  const rename = useRenameTask();

  const save = () => {
    if (rename.isPending) return;
    const title = newTitle(current, typed);
    if (title === null) {
      navigation.goBack();
      return;
    }
    rename.mutate({ task: id, title }, { onSuccess: () => navigation.goBack() });
  };

  return (
    <Screen scroll>
      <View style={{ gap: spacing.sm }}>
        <TextField
          value={typed}
          onChangeText={setTyped}
          placeholder="The task's title"
          accessibilityLabel="Title"
          returnKeyType="done"
          onSubmitEditing={save}
          autoFocus
          selectTextOnFocus
        />
        {rename.isError ? (
          <Text variant="footnote" tone="danger" accessibilityLiveRegion="polite">
            {renameFailure(rename.error)}
          </Text>
        ) : null}
        <Button label="Save" onPress={save} loading={rename.isPending} block />
      </View>
    </Screen>
  );
}
