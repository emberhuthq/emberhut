import { useNavigation } from "@react-navigation/native";
import Constants from "expo-constants";
import { Alert, View } from "react-native";
import { aboutYouTitle } from "@/features/aboutYou/facts";
import { emberCalled } from "@/features/ember/name";
import { LookPicker } from "@/features/ember/LookPicker";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { sessionStore } from "@/services";
import { offlineReason } from "@/state/connection";
import { useConnection, useSession } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, Chip, ListRow, Screen, Section, Text, usePullRefresh } from "@/ui";
import { Appearance, RoomsList } from "./Appearance";
import { Devices, PairAnother } from "./Devices";
import { EmberHero } from "./EmberHero";
import { calledIn, groupStatus, type GroupId, type Now } from "./entries";
import { Environments } from "./Environments";
import { FallbackServer } from "./FallbackServer";
import { PushRow } from "./PushRow";
import { Switches } from "./Switches";
import { useSettingsNow } from "./useSettingsNow";

/** Where the agent's model sends what it reads, said once on Privacy. */
const dataFlow =
  "Your messages, and what your ember reads to answer them (facts about you, mail, pages), go to the model you picked. " +
  "Gmail and Slack are read from their own servers. Phone alerts pass through Apple's push service.";

/** One group of Settings on a screen of its own, built of the rows Settings had: how it stands first, then its rows. */
export function SettingsGroupScreen({ route }: RootScreenProps<"SettingsGroup">) {
  const { spacing } = useTheme();
  const { group, focus } = route.params;
  const now = useSettingsNow();
  const state = useEmberhutState();
  const pull = usePullRefresh(state.refetch);
  // Your ember's card says how it stands, in the same words: its screen doesn't say it again above.
  const status = group === "ember" ? "" : groupStatus(group, now);
  return (
    <Screen scroll {...pull}>
      <View style={{ gap: spacing.lg }}>
        {status ? (
          <Text variant="footnote" tone="secondary">
            {status}
          </Text>
        ) : null}
        <GroupRows group={group} now={now} renaming={focus === "ember-name"} />
      </View>
    </Screen>
  );
}

function GroupRows({ group, now, renaming }: { group: GroupId; now: Now; renaming: boolean }) {
  switch (group) {
    case "ember":
      return (
        <>
          <EmberHero now={now} renaming={renaming} />
          <OpensRows now={now} />
          <LookSection />
        </>
      );
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

/** Your ember's look, below its card and what it opens: the same theme as Appearance. */
function LookSection() {
  const { spacing } = useTheme();
  return (
    <Section title="Look" subtitle="Also in Appearance. Syncs to your other devices.">
      <View key="look" style={{ padding: spacing.md }}>
        <LookPicker />
      </View>
    </Section>
  );
}

/** What it thinks with and what it knows, each called by the ember's name: rows that open their screens. */
function OpensRows({ now }: { now: Now }) {
  const navigate = useNavigate();
  const called = calledIn(now);
  const knows = now.facts === undefined ? "Facts, and lessons from your corrections" : `${now.facts} things`;
  return (
    <>
      <Section title={`What ${called} thinks with`}>
        <ListRow
          key="thinks"
          icon="key-outline"
          title="Model provider"
          subtitle={now.provider ?? "Looking…"}
          onPress={() => navigate("Credentials")}
        />
      </Section>
      <Section title={`What ${called} knows`}>
        <ListRow
          key="about"
          icon="book-outline"
          title="About you"
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
  const called = emberCalled(useEmberhutState().data);
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
      <Section title={`Computers ${called} works on`}>
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
  const called = emberCalled(useEmberhutState().data);
  return (
    <Section footer="Today sharing is set up through your server's API, for developers. Inviting someone from here comes next.">
      <ListRow
        key="only"
        icon="people-outline"
        title={`Only you can talk to ${called}`}
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

/** Advanced: where the ember lives, the fallback server, diagnostics, this app's version, where your ember works, signing this phone out. */
function AdvancedRows() {
  const signedIn = useSession((s) => s.status === "signedIn");
  const serverUrl = useSession((s) => s.serverUrl);
  const machineId = useSession((s) => s.machineId);
  const state = useEmberhutState();
  const offline = useConnection(offlineReason);
  const version = Constants.expoConfig?.version ?? "?";
  const build = Constants.expoConfig?.android?.versionCode;
  const signOut = () =>
    Alert.alert("Sign out this phone?", "This phone forgets its session. You'll need a new pairing code to sign in again.", [
      { text: "Cancel", style: "cancel" },
      { text: "Sign out", style: "destructive", onPress: () => inBackground(sessionStore.getState().signOut(), "Settings: signOut") },
    ]);
  const mode = state.data?.auth?.mode;
  const called = emberCalled(state.data);
  return (
    <>
      <Section title={`Where ${called} lives`}>
        <ListRow key="server" icon="server-outline" title="Server" subtitle={serverUrl ?? "None yet: it's set when this phone pairs"} />
        <ListRow key="version" icon="information-circle-outline" title="App version" subtitle={build ? `${version} (${build})` : version} />
      </Section>
      {signedIn ? <FallbackServer /> : null}
      <Section title="Diagnostics">
        <ListRow
          key="connection"
          icon={offline ? "cloud-offline-outline" : "cloud-done-outline"}
          title={offline ? `Can't reach ${called}` : state.data ? "Connected" : `Reaching ${called}…`}
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
