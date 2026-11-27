import { OWNER } from "@/api";
import { accountConfig } from "@/config/account";
import { statusText } from "@/features/browser/browser";
import { useBrowserDevice } from "@/features/browser/browserDevice";
import { todayLists } from "@/features/tasks/model";
import type { TabScreenProps } from "@/navigation/types";
import { useActions } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";
import { ListRow, Screen, Section } from "@/ui";

/**
 * Everything without a tab of its own: Today, About you, Emberhut's browser, the rooms, the machines, Party and Settings, and
 * the emberhut.com account in builds that have it.
 */
export function MoreScreen({ navigation }: TabScreenProps<"More">) {
  const state = useEmberhutState();
  const gone = useActions((s) => s.gone);
  // What the Today page lists, each list counted under its own name.
  const lists = todayLists(state.data, gone);
  const todayLine =
    lists.decide.length + lists.ranked.length === 0
      ? "What to decide today, then Emberhut's ranked list"
      : `${lists.decide.length} to decide today · ${lists.ranked.length} ranked`;
  const device = useBrowserDevice();
  const browser = state.data?.browser;
  // Handed to you, it says so as the browser page does (asked to take over, you're driving, your other device).
  const browserLine =
    browser?.control === OWNER
      ? statusText(browser, device)
      : browser?.driver != null
        ? "Emberhut is using it now"
        : "Watch it, or take it over";
  return (
    <Screen scroll>
      <Section>
        <ListRow key="today" icon="today-outline" title="Today" subtitle={todayLine} onPress={() => navigation.navigate("Today")} />
        <ListRow
          key="about"
          icon="person-outline"
          title="About you"
          subtitle="What Emberhut knows about you, and the rules it's trying out"
          onPress={() => navigation.navigate("AboutYou")}
        />
      </Section>
      <Section>
        <ListRow
          key="browser"
          icon="globe-outline"
          title="Emberhut's browser"
          subtitle={browserLine}
          onPress={() => navigation.navigate("Browser")}
        />
        <ListRow
          key="rooms"
          icon="albums-outline"
          title="Rooms"
          subtitle="Main and a room per project"
          onPress={() => navigation.navigate("Rooms")}
        />
        <ListRow
          key="machines"
          icon="hardware-chip-outline"
          title="Machines"
          subtitle="Your runner machines: CPU, memory, GPUs, disks, sessions"
          onPress={() => navigation.navigate("Machines")}
        />
        <ListRow
          key="party"
          icon="people-outline"
          title="Party"
          subtitle="AI personas talking to each other about your topic"
          onPress={() => navigation.navigate("Parties")}
        />
      </Section>
      <Section>
        <ListRow
          key="settings"
          icon="settings-outline"
          title="Settings"
          subtitle="Look, notifications, devices, Emberhut's servers and skills"
          onPress={() => navigation.navigate("Settings")}
        />
        {accountConfig ? (
          <ListRow
            key="account"
            icon="person-circle-outline"
            title="Account"
            subtitle="Your emberhut.com account and the machines it finds"
            onPress={() => navigation.navigate("Account")}
          />
        ) : null}
      </Section>
    </Screen>
  );
}
