import Constants from "expo-constants";
import { Alert } from "react-native";
import { inBackground } from "@/log";
import { sessionStore } from "@/services";
import { offlineReason } from "@/state/connection";
import { useConnection, useSession } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";
import { Button, ListRow, Screen, Section, usePullRefresh } from "@/ui";
import { Appearance } from "./Appearance";
import { Devices, PairAnother } from "./Devices";
import { Environments } from "./Environments";
import { FallbackServer } from "./FallbackServer";
import { PushRow } from "./PushRow";
import { Switches } from "./Switches";

/**
 * Settings: Emberhut's look (kept on the server), notifications and Emberhut's server; your signed-in devices and pairing another one; then, as on
 * the reference client's page, Emberhut's MCP servers and skills with your switches and the sources and machines it depends on.
 */
export function SettingsScreen() {
  const signedIn = useSession((s) => s.status === "signedIn");
  const serverUrl = useSession((s) => s.serverUrl);
  const machineId = useSession((s) => s.machineId);
  const state = useEmberhutState();
  const pull = usePullRefresh(state.refetch);
  const offline = useConnection(offlineReason);
  const version = Constants.expoConfig?.version ?? "?";
  const build = Constants.expoConfig?.android?.versionCode;
  const signOut = () =>
    Alert.alert("Sign out of Emberhut?", "This phone forgets its session. You'll need a new pairing code to sign in again.", [
      { text: "Cancel", style: "cancel" },
      { text: "Sign out", style: "destructive", onPress: () => inBackground(sessionStore.getState().signOut(), "SettingsScreen: signOut") },
    ]);
  return (
    <Screen scroll {...pull}>
      <Appearance />
      <Section title="This phone">
        {signedIn ? <PushRow key="push" /> : null}
        <ListRow key="server" icon="server-outline" title="Server" subtitle={serverUrl ?? "None yet: it's set when this phone pairs"} />
        {machineId ? (
          // The id its key proved at pairing, to compare with the one the server prints.
          <ListRow key="machine" icon="finger-print-outline" title="Machine id" subtitle={machineId} />
        ) : null}
        <ListRow
          key="connection"
          icon={offline ? "cloud-offline-outline" : "cloud-done-outline"}
          title={
            offline
              ? "Can't reach Emberhut"
              : state.data
                ? state.data.on
                  ? "Connected · Emberhut is on"
                  : "Connected · Emberhut is paused"
                : "Reaching Emberhut…"
          }
          subtitle={offline ?? undefined}
        />
        <ListRow key="version" icon="information-circle-outline" title="App version" subtitle={build ? `${version} (${build})` : version} />
      </Section>
      {signedIn ? <FallbackServer /> : null}
      <Devices auth={state.data?.auth} />
      {signedIn ? <PairAnother /> : null}
      <Switches />
      <Environments />
      {signedIn ? <Button label="Sign out" variant="destructive" icon="log-out-outline" onPress={signOut} /> : null}
    </Screen>
  );
}
