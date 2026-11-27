import { Linking } from "react-native";
import { inBackground } from "@/log";
import { pushStore } from "@/services";
import { usePush } from "@/state/hooks";
import { Button, ListRow } from "@/ui";

/** Whether Emberhut can reach this phone with the app closed, and what to do when it can't. */
export function PushRow() {
  const permission = usePush((s) => s.permission);
  const registered = usePush((s) => s.registered);
  const error = usePush((s) => s.error);
  if (permission === "denied") {
    return (
      <ListRow
        icon="notifications-off-outline"
        title="Notifications are off"
        subtitle="Emberhut can't reach this phone with the app closed. Turn them on in the phone's settings for this app."
        trailing={
          <Button
            label="Open"
            variant="plain"
            onPress={() => inBackground(Linking.openSettings(), "PushRow: openSettings")}
            accessibilityLabel="Open the phone's settings for this app"
          />
        }
      />
    );
  }
  if (registered)
    return <ListRow icon="notifications-outline" title="Notifications" subtitle="On: Emberhut reaches this phone with the app closed" />;
  return (
    <ListRow
      icon="notifications-outline"
      title="Notifications"
      subtitle={error ? `Not set up: ${error}` : "Setting up…"}
      trailing={<Button label="Retry" variant="plain" onPress={() => inBackground(pushStore.getState().sync(), "PushRow: sync")} />}
    />
  );
}
