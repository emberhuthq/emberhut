import { createNativeBottomTabNavigator } from "@bottom-tabs/react-navigation";
import { Platform, type ImageSourcePropType } from "react-native";
import type { AppleIcon } from "react-native-bottom-tabs";
import { ChatScreen } from "@/features/chat/ChatScreen";
import { MoreScreen } from "@/features/more/MoreScreen";
import { NeedsYouScreen } from "@/features/needsYou/NeedsYouScreen";
import { useNeedsCount } from "@/features/needsYou/useNeedsCount";
import { SessionsScreen } from "@/features/sessions/SessionsScreen";
import { TasksScreen } from "@/features/tasks/TasksScreen";
import { useTheme } from "@/theme";
import { TranslucentBars } from "@/ui";
import { withHeader } from "./TabHeader";
import { androidTabIcon } from "./tabIcons";
import type { TabParamList } from "./types";

// Each platform's own tab bar around the same screens (react-native-bottom-tabs): UIKit's UITabBarController on iOS,
// Liquid Glass from iOS 26 and the standard blur before, with SF Symbols; Material 3's bottom navigation on Android,
// with Material icons (bundled PNGs, tabIcons.ts) and its pill behind the one picked. Each tab has a native header of its own (TabHeader.tsx).
const Tab = createNativeBottomTabNavigator<TabParamList>();

type Tabbed = keyof TabParamList;

/** SF Symbols, the plain one and the filled one for the tab picked. */
const symbols: Record<Tabbed, [string, string]> = {
  Chat: ["bubble.left.and.bubble.right", "bubble.left.and.bubble.right.fill"],
  NeedsYou: ["hand.raised", "hand.raised.fill"],
  Tasks: ["checklist", "checklist"],
  Sessions: ["terminal", "terminal.fill"],
  More: ["ellipsis.circle", "ellipsis.circle.fill"],
};

// The chat's list is drawn upside down, so iOS 26's scroll-edge fade would cover it whole: react-native-screens
// (4.30.1 here; native-stack 7.21's scrollEdgeEffects option, set on RNSScreen) turns it off for the screen's content
// scroll view, and the chat's header is the solid standard bar.
const Chat = withHeader(ChatScreen, { largeTitle: false, solid: true, screen: { scrollEdgeEffects: { top: "hidden", bottom: "hidden" } } });
const NeedsYou = withHeader(NeedsYouScreen, { largeTitle: true });
const Tasks = withHeader(TasksScreen, { largeTitle: true });
const Sessions = withHeader(SessionsScreen, { largeTitle: true });
const More = withHeader(MoreScreen, { largeTitle: true });

export function Tabs() {
  const { colors } = useTheme();
  const needs = useNeedsCount();
  const icon =
    (tab: Tabbed) =>
    ({ focused }: { focused: boolean }): ImageSourcePropType | AppleIcon => {
      if (Platform.OS === "ios") return { sfSymbol: symbols[tab][focused ? 1 : 0] as AppleIcon["sfSymbol"] };
      return androidTabIcon(tab, focused);
    };
  return (
    <TranslucentBars.Provider value={Platform.OS === "ios"}>
      <Tab.Navigator
        hapticFeedbackEnabled
        // Every tab's label on Android too (Material's default with five shows only the picked one's), as on iOS.
        labeled
        tabBarActiveTintColor={colors.accent}
        tabBarInactiveTintColor={colors.textTertiary}
        activeIndicatorColor={colors.surfaceRaised}
        tabBarStyle={Platform.OS === "android" ? { backgroundColor: colors.surface } : undefined}
        screenOptions={{ sceneStyle: { backgroundColor: colors.background } }}
      >
        <Tab.Screen name="Chat" component={Chat} options={{ title: "Emberhut", tabBarIcon: icon("Chat") }} />
        <Tab.Screen
          name="NeedsYou"
          component={NeedsYou}
          options={{ title: "Needs you", tabBarIcon: icon("NeedsYou"), tabBarBadge: needs > 0 ? String(needs) : undefined }}
        />
        <Tab.Screen name="Tasks" component={Tasks} options={{ title: "Tasks", tabBarIcon: icon("Tasks") }} />
        <Tab.Screen name="Sessions" component={Sessions} options={{ title: "Sessions", tabBarIcon: icon("Sessions") }} />
        <Tab.Screen name="More" component={More} options={{ title: "More", tabBarIcon: icon("More") }} />
      </Tab.Navigator>
    </TranslucentBars.Provider>
  );
}
