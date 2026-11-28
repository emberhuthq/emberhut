import { useNavigation } from "@react-navigation/native";
import Constants from "expo-constants";
import { Alert, Linking, Platform, View } from "react-native";
import { aboutYouTitle } from "@/features/aboutYou/facts";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { sessionStore } from "@/services";
import { offlineReason } from "@/state/connection";
import { useConnection, useSession } from "@/state/hooks";
import { useApplySettings, useEmberhutState } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, Chip, ListRow, Screen, Section, Text, Toggle, usePullRefresh } from "@/ui";
import { Appearance, RoomsList } from "./Appearance";
import { Devices, PairAnother } from "./Devices";
import { groupStatus, type GroupId, type Now } from "./entries";
import { Environments } from "./Environments";
import { FallbackServer } from "./FallbackServer";
import { PushRow } from "./PushRow";
import { Switches } from "./Switches";
import { storeLink, versionLine } from "./updates";
import { useSettingsNow } from "./useSettingsNow";

/** Where the agent's model sends what it reads, said once on Privacy. */
const dataFlow =
  "Your messages, and what your ember reads to answer them (facts about you, mail, pages), go to the model you picked. " +
  "Gmail and Slack are read from their own servers. Phone alerts pass through Apple's push service.";

/** One group of Settings on a screen of its own, built of the rows Settings had: how it stands first, then its rows. */
export function SettingsGroupScreen({ route }: RootScreenProps<"SettingsGroup">) {
  const { spacing } = useTheme();
  const { group } = route.params;
  const now = useSettingsNow();
  const state = useEmberhutState();
  const pull = usePullRefresh(state.refetch);
  const status = groupStatus(group, now);
  return (
    <Screen scroll {...pull}>
      <View style={{ gap: spacing.lg }}>
        {status ? (
          <Text variant="footnote" tone="secondary">
            {status}
          </Text>
        ) : null}
        <GroupRows group={group} now={now} />
      </View>
    </Screen>
  );
}

function GroupRows({ group, now }: { group: GroupId; now: Now }) {
  switch (group) {
    case "ember":
      return <EmberRows now={now} />;
    case "appearance":
      return (
        <>
          <Appearance />
          <RoomsList />
        </>
      );
    case "devices":
      return <DevicesRows />;
    case "connections":
      return (
        <>
          <Environments show="sources" />
          <Switches />
        </>
      );
    case "sharing":
      return <SharingRows />;
    case "privacy":
      return <PrivacyRows now={now} />;
    case "advanced":
      return <AdvancedRows />;
    case "account":
      return <Text tone="secondary">Your emberhut.com account opens from More.</Text>;
  }
}

/** Your ember: on duty (with how it's reached), what it thinks with, what it knows. */
function EmberRows({ now }: { now: Now }) {
  const settings = useApplySettings();
  const offline = useConnection(offlineReason);
  return (
    <>
      <Section>
        <ListRow
          key="duty"
          icon="flame-outline"
          title={now.onDuty ? "On duty" : "Off duty"}
          subtitle={
            offline
              ? "Can't reach your ember"
              : now.onDuty
                ? "Works on its own, and answers when you message it"
                : "Still answers when you message it"
          }
          trailing={
            <Toggle
              value={now.onDuty}
              onChange={(on) => settings.mutate({ on })}
              disabled={settings.isPending || offline !== null}
              label="On duty"
            />
          }
        />
      </Section>
      <OpensRows now={now} />
    </>
  );
}

/** What it thinks with and what it knows: rows that open their screens. */
function OpensRows({ now }: { now: Now }) {
  const navigate = useNavigate();
  const knows = now.facts === undefined ? "Facts, and lessons from your corrections" : `${now.facts} things`;
  return (
    <>
      <Section title="What it thinks with">
        <ListRow
          key="thinks"
          icon="key-outline"
          title="What your ember thinks with"
          subtitle={now.provider ?? "Looking…"}
          onPress={() => navigate("Credentials")}
        />
      </Section>
      <Section title="What it knows">
        <ListRow
          key="about"
          icon="book-outline"
          title={aboutYouTitle}
          subtitle={now.lessons > 0 ? `${knows} · ${now.lessons} lessons to review` : knows}
          onPress={() => navigate("AboutYou")}
        />
      </Section>
    </>
  );
}

/** Opens one of the screens a group's rows lead to. */
function useNavigate() {
  const navigation = useNavigation();
  return (screen: "Credentials" | "AboutYou" | "Machines") => navigation.navigate(screen);
}

/** Devices: this phone's alerts, the signed-in devices, pairing another, the machines. */
function DevicesRows() {
  const signedIn = useSession((s) => s.status === "signedIn");
  const state = useEmberhutState();
  const navigate = useNavigate();
  return (
    <>
      {signedIn ? (
        <Section title="This phone">
          <PushRow key="push" />
        </Section>
      ) : null}
      <Devices auth={state.data?.auth} />
      {signedIn ? <PairAnother /> : null}
      <Section title="Computers your ember works on">
        <ListRow
          key="machines"
          icon="hardware-chip-outline"
          title="Machines"
          subtitle="CPU, memory, GPUs, disks, sessions"
          onPress={() => navigate("Machines")}
        />
      </Section>
    </>
  );
}

/** Sharing: only you, today, and how it is set up for now. */
function SharingRows() {
  return (
    <Section footer="Today sharing is set up through your server's API, for developers. Inviting someone from here comes next.">
      <ListRow
        key="only"
        icon="people-outline"
        title="Only you can talk to your ember"
        subtitle="Sharing lets someone else ask it things, within limits you set."
        trailing={<Chip label="Early preview" />}
      />
    </Section>
  );
}

/** Privacy: what your ember knows, and where your data goes. */
function PrivacyRows({ now }: { now: Now }) {
  const navigate = useNavigate();
  return (
    <Section footer={dataFlow}>
      <ListRow
        key="about"
        icon="book-outline"
        title={aboutYouTitle}
        subtitle={now.facts === undefined ? "Fix or forget any of it" : `${now.facts} things · fix or forget any`}
        onPress={() => navigate("AboutYou")}
      />
    </Section>
  );
}

/** Advanced: your ember's home, the fallback server, diagnostics, this app's version and the store its updates come through, where your ember works, signing this phone out. */
function AdvancedRows() {
  const signedIn = useSession((s) => s.status === "signedIn");
  const serverUrl = useSession((s) => s.serverUrl);
  const machineId = useSession((s) => s.machineId);
  const state = useEmberhutState();
  const offline = useConnection(offlineReason);
  const config = Constants.expoConfig;
  const build = Platform.OS === "ios" ? config?.ios?.buildNumber : config?.android?.versionCode;
  const store = storeLink(Platform.OS);
  const openStore = () =>
    inBackground(
      Linking.openURL(store.url).catch(() => Linking.openURL(store.fallback)),
      "Settings: open the store",
    );
  const signOut = () =>
    Alert.alert("Sign out this phone?", "This phone forgets its session. You'll need a new pairing code to sign in again.", [
      { text: "Cancel", style: "cancel" },
      { text: "Sign out", style: "destructive", onPress: () => inBackground(sessionStore.getState().signOut(), "Settings: signOut") },
    ]);
  const mode = state.data?.auth?.mode;
  return (
    <>
      <Section title="Your ember's home">
        <ListRow key="server" icon="server-outline" title="Server" subtitle={serverUrl ?? "None yet: it's set when this phone pairs"} />
        <ListRow key="version" icon="information-circle-outline" title="App version" subtitle={versionLine(config?.version, build)} />
        <ListRow key="store" icon="download-outline" title={store.title} onPress={openStore} />
      </Section>
      {signedIn ? <FallbackServer /> : null}
      <Section title="Diagnostics">
        <ListRow
          key="connection"
          icon={offline ? "cloud-offline-outline" : "cloud-done-outline"}
          title={offline ? "Can't reach your ember" : state.data ? "Connected" : "Reaching your ember…"}
          subtitle={offline ?? undefined}
        />
        {mode ? (
          <ListRow
            key="mode"
            icon="lock-closed-outline"
            title="Sign-in mode"
            subtitle={mode === "enforce" ? "Required" : "Not required yet"}
          />
        ) : null}
        {machineId ? (
          // The id its key proved at pairing, to compare with the one the server prints.
          <ListRow key="machine" icon="finger-print-outline" title="Machine id" subtitle={machineId} />
        ) : null}
      </Section>
      <Environments show="workplaces" />
      {signedIn ? <Button label="Sign out" variant="destructive" icon="log-out-outline" onPress={signOut} /> : null}
    </>
  );
}
