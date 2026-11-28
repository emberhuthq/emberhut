import type { LinkingOptions } from "@react-navigation/native";
import { createNativeStackNavigator } from "@react-navigation/native-stack";
import { Platform } from "react-native";
import { linkSchemes } from "@/config";
import { accountConfig } from "@/config/account";
import { AboutYouScreen } from "@/features/aboutYou/AboutYouScreen";
import { ApprovalScreen } from "@/features/approvals/ApprovalScreen";
import { ApprovalsScreen } from "@/features/approvals/ApprovalsScreen";
import { FactScreen } from "@/features/aboutYou/FactScreen";
import { aboutYouTitle } from "@/features/aboutYou/facts";
import { AddServerScreen, SignInScreen } from "@/features/auth/SignInScreen";
import { BrowserScreen } from "@/features/browser/BrowserScreen";
import { RoomsScreen } from "@/features/chat/RoomsScreen";
import { CredentialsScreen } from "@/features/ember/CredentialsScreen";
import { FileScreen } from "@/features/files/FileScreen";
import { MachinesScreen } from "@/features/machines/MachinesScreen";
import { NewPartyScreen } from "@/features/party/NewPartyScreen";
import { PartiesScreen } from "@/features/party/PartiesScreen";
import { PartyScreen } from "@/features/party/PartyScreen";
import { NewSessionScreen } from "@/features/sessions/NewSessionScreen";
import { MoveToTaskScreen } from "@/features/sessions/MoveToTaskScreen";
import { SessionScreen } from "@/features/sessions/SessionScreen";
import { SessionsScreen } from "@/features/sessions/SessionsScreen";
import { groupName } from "@/features/settings/entries";
import { SettingsGroupScreen } from "@/features/settings/SettingsGroupScreen";
import { SettingsScreen } from "@/features/settings/SettingsScreen";
import { AreaScreen } from "@/features/tasks/AreaScreen";
import { ProjectScreen } from "@/features/tasks/ProjectScreen";
import { TaskScreen } from "@/features/tasks/TaskScreen";
import { TodayScreen } from "@/features/tasks/TodayScreen";
import { notificationLinking } from "@/push/native";
import { useSession } from "@/state/hooks";
import { needsSignIn } from "@/state/session";
import { useTheme } from "@/theme";
import { Tabs } from "./Tabs";
import type { RootStackParamList } from "./types";

const Stack = createNativeStackNavigator<RootStackParamList>();

/**
 * The account's screen, in builds with the account on; null in the others. Chosen on the literal env var, as
 * AccountProvider is, so that in builds without the account Clerk (which the screen uses) isn't bundled at all.
 */
const AccountScreen = process.env.EXPO_PUBLIC_EMBERHUT_CLERK_PUBLISHABLE_KEY
  ? // eslint-disable-next-line @typescript-eslint/no-require-imports -- a require the bundler can drop, unlike an import
    (require("../features/account/AccountScreen") as typeof import("../features/account/AccountScreen")).AccountScreen
  : null;

/**
 * emberhut://chat?room=4 is what a push opens; tasks, sessions, parties (emberhut://party, emberhut://party/3) and approvals
 * (emberhut://approvals, emberhut://approval/7) have links of their own.
 */
export const linking: LinkingOptions<RootStackParamList> = {
  prefixes: [...linkSchemes],
  ...notificationLinking,
  config: {
    screens: {
      Tabs: { screens: { Chat: { path: "chat", parse: { room: Number } } } },
      Task: { path: "task/:id", parse: { id: Number } },
      Project: { path: "project/:id", parse: { id: Number } },
      Session: "session/:id",
      Browser: "browser",
      Machines: "machines",
      Parties: "party",
      Party: { path: "party/:id", parse: { id: Number } },
      Approvals: "approvals",
      Approval: { path: "approval/:id", parse: { id: Number } },
    },
  },
};

export function RootStack() {
  const { colors } = useTheme();
  const signIn = useSession(needsSignIn);
  // The account's screen, in builds that have one: from More, and from sign-in to find a machine to pair with.
  // Registered only when the account is on too: a key of white space alone requires the screen but mounts no Clerk.
  const account =
    AccountScreen && accountConfig ? <Stack.Screen name="Account" component={AccountScreen} options={{ title: "Account" }} /> : null;
  return (
    <Stack.Navigator
      screenOptions={{
        headerStyle: { backgroundColor: colors.surface },
        headerTintColor: colors.text,
        contentStyle: { backgroundColor: colors.background },
        // iOS would title the back button after the screen below ("Tabs" from a task): the arrow alone says it.
        headerBackButtonDisplayMode: "minimal",
      }}
    >
      {signIn ? (
        <>
          <Stack.Screen name="SignIn" component={SignInScreen} options={{ headerShown: false }} />
          {account}
        </>
      ) : (
        <>
          <Stack.Screen name="Tabs" component={Tabs} options={{ headerShown: false }} />
          <Stack.Screen
            name="Rooms"
            component={RoomsScreen}
            // A sheet from the bottom at 60% of the screen that pulls up to full height.
            options={{
              title: "Rooms",
              presentation: "formSheet",
              headerShown: false,
              sheetAllowedDetents: [0.6, 1],
              sheetGrabberVisible: true,
              sheetCornerRadius: 28,
              // iOS: the sheet's own material shows through (Liquid Glass on iOS 26); the rows stay readable on it.
              // Android's sheet has no material of its own: the color here is its surface (rounded at the top), so it
              // must be solid or the chat shows through the rows.
              contentStyle: { backgroundColor: Platform.OS === "ios" ? "transparent" : colors.surface },
            }}
          />
          <Stack.Screen name="Task" component={TaskScreen} options={{ title: "Task" }} />
          <Stack.Screen name="Project" component={ProjectScreen} options={{ title: "Project" }} />
          <Stack.Screen name="Area" component={AreaScreen} options={{ title: "Area" }} />
          <Stack.Screen name="Today" component={TodayScreen} options={{ title: "Today" }} />
          <Stack.Screen name="Session" component={SessionScreen} options={({ route }) => ({ title: route.params.title ?? "Session" })} />
          <Stack.Screen name="SessionList" component={SessionsScreen} options={{ title: "Sessions" }} />
          <Stack.Screen name="NewSession" component={NewSessionScreen} options={{ title: "New session", presentation: "modal" }} />
          <Stack.Screen name="MoveToTask" component={MoveToTaskScreen} options={{ title: "Move to task", presentation: "modal" }} />
          <Stack.Screen
            name="Browser"
            component={BrowserScreen}
            options={{ title: "Emberhut's browser", presentation: "fullScreenModal", headerShown: false }}
          />
          <Stack.Screen
            name="File"
            component={FileScreen}
            // A page sheet: it swipes down to close, and its header has Done (FileScreen).
            options={({ route }) => ({ title: route.params.name, presentation: "modal" })}
          />
          <Stack.Screen name="AboutYou" component={AboutYouScreen} options={{ title: aboutYouTitle }} />
          <Stack.Screen name="Fact" component={FactScreen} options={{ title: "Fix a fact", presentation: "modal" }} />
          <Stack.Screen name="Settings" component={SettingsScreen} options={{ title: "Settings" }} />
          <Stack.Screen name="AddServer" component={AddServerScreen} options={{ title: "Pair with a server", presentation: "modal" }} />
          <Stack.Screen
            name="SettingsGroup"
            component={SettingsGroupScreen}
            options={({ route }) => ({ title: groupName[route.params.group] })}
          />
          <Stack.Screen name="Credentials" component={CredentialsScreen} options={{ title: "What your ember thinks with" }} />
          <Stack.Screen name="Machines" component={MachinesScreen} options={{ title: "Machines" }} />
          <Stack.Screen name="Parties" component={PartiesScreen} options={{ title: "Party" }} />
          <Stack.Screen name="Party" component={PartyScreen} options={({ route }) => ({ title: route.params.title ?? "Party" })} />
          <Stack.Screen name="NewParty" component={NewPartyScreen} options={{ title: "New party", presentation: "modal" }} />
          {account}
          <Stack.Screen name="Approvals" component={ApprovalsScreen} options={{ title: "Approvals" }} />
          <Stack.Screen name="Approval" component={ApprovalScreen} options={{ title: "Approval" }} />
        </>
      )}
    </Stack.Navigator>
  );
}
