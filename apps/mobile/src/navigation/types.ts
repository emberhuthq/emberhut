import type { NativeBottomTabScreenProps } from "@bottom-tabs/react-navigation";
import type { CompositeScreenProps, NavigatorScreenParams } from "@react-navigation/native";
import type { NativeStackNavigationOptions, NativeStackScreenProps } from "@react-navigation/native-stack";
import type { ChatFile } from "@/api";
import type { GroupId as SettingsGroupId } from "@/features/settings/entries";

export type TabParamList = {
  /** A room's chat; without `room`, the one last open. `draft` starts the message box with it (Talk on a task). */
  Chat: { room?: number; draft?: string } | undefined;
  NeedsYou: undefined;
  /** Today, the projects and their tasks. */
  Tasks: undefined;
  /** Your own sessions and the runner's. Not a tab while the server keeps task plans: SessionList, from More, instead. */
  Sessions: undefined;
  /** What your ember knows about you, settings, the browser. */
  More: undefined;
};

export type RootStackParamList = {
  SignIn: undefined;
  /** Pairing with another server: a new one, this one kept as the fallback ("switch"), or a fallback server. */
  AddServer: { role: "switch" | "fallback" };
  /** The emberhut.com account: sign in, its machines (only in builds with the account on). */
  Account: undefined;
  Tabs: NavigatorScreenParams<TabParamList> | undefined;
  /** The room switcher. */
  Rooms: undefined;
  Task: { id: number };
  Project: { id: number };
  Area: { id: number };
  Today: undefined;
  /** A session's transcript ("r-…" runner, "w-<task>" worker); `agent` is a sub-agent's call id: only its own steps. */
  Session: { id: string; agent?: string; title?: string };
  NewSession: undefined;
  /**
   * Move to task…: the open task a session's next turns work on (a server that keeps plans). `tasks`: the ones it works
   * on now, left out of the list; `title` shows while the state hasn't the session.
   */
  MoveToTask: { session: string; title?: string; tasks?: number[] };
  /** Rename, from a task page's menu: `title` shows while the state hasn't the task. */
  RenameTask: { id: number; title: string };
  /** The Sessions list on a page of its own, from More, while the server keeps task plans and Tasks are the main surface. */
  SessionList: undefined;
  /** Emberhut's browser, live, and taking it over. */
  Browser: undefined;
  /** A server file full screen: a picture, a video, a PDF, an HTML report. */
  File: { path: string; name: string; kind: ChatFile["kind"] };
  AboutYou: undefined;
  /** One fact Emberhut keeps about you, to fix or forget ("people/x"). */
  Fact: { id: string };
  /** Settings' front door: the search, the groups, the suggestions. */
  Settings: undefined;
  /** One group of Settings ("ember", "connections"), on a screen of its own. */
  /** A group of Settings; `focus`: a setting on it to open at once (Your ember's name, to type a new one). */
  SettingsGroup: { group: SettingsGroupId; focus?: "ember-name" };
  /** What your ember thinks with: the model provider, its key and model, checked with the provider before it's saved. */
  Credentials: undefined;
  /** Each runner machine's live capacity, as the server lists them. */
  Machines: undefined;
  /** Your parties: AI personas talking to each other. */
  Parties: undefined;
  /** One party, live; `title` shows until it loads. */
  Party: { id: number; title?: string };
  NewParty: undefined;
  /** Pending approvals: what waits on you, then what waits on an approver; with `agent`, only that agent's. */
  Approvals: { agent?: string } | undefined;
  /** One approval request, its whole payload, and Approve or Reject when it waits on you. */
  Approval: { id: number };
};

export type RootScreenProps<Name extends keyof RootStackParamList> = NativeStackScreenProps<RootStackParamList, Name>;

type TabProps<Name extends keyof TabParamList> = CompositeScreenProps<
  NativeBottomTabScreenProps<TabParamList, Name>,
  NativeStackScreenProps<RootStackParamList>
>;

/** A tab's screen: its tab's route and navigation, whose setOptions sets its own native header (TabHeader.tsx). */
export type TabScreenProps<Name extends keyof TabParamList> = Omit<TabProps<Name>, "navigation"> & {
  navigation: Omit<TabProps<Name>["navigation"], "setOptions"> & {
    setOptions: (options: Partial<NativeStackNavigationOptions>) => void;
  };
};

declare global {
  namespace ReactNavigation {
    // eslint-disable-next-line @typescript-eslint/no-empty-object-type -- React Navigation types useNavigation() through this empty interface
    interface RootParamList extends RootStackParamList {}
  }
}
