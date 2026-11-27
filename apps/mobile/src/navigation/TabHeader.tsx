import type { ParamListBase } from "@react-navigation/native";
import {
  createNativeStackNavigator,
  type NativeStackNavigationOptions,
  type NativeStackNavigationProp,
} from "@react-navigation/native-stack";
import { useMemo, type ComponentType } from "react";
import { Platform } from "react-native";
import { useTheme } from "@/theme";
import type { TabParamList, TabScreenProps } from "./types";

/**
 * A tab's screen under a native header: UIKit's navigation bar on iOS (a large title where Apple's own apps have one,
 * its own material over what scrolls under it), Material's top app bar on Android. The screen keeps its tab's route and navigation
 * (its params, the room a link opens), and only its header options go to the stack that draws the header.
 */
export function withHeader<Name extends keyof TabParamList>(
  Screen: ComponentType<TabScreenProps<Name>>,
  {
    largeTitle,
    solid = false,
    screen = {},
  }: {
    largeTitle: boolean;
    solid?: boolean;
    /** The stack screen's own options, beyond the header: for example its scroll-edge effects. */
    screen?: NativeStackNavigationOptions;
  },
): ComponentType<TabScreenProps<Name>> {
  const Stack = createNativeStackNavigator();
  function TabWithHeader(tab: TabScreenProps<Name>) {
    const { colors } = useTheme();
    const ios = Platform.OS === "ios";
    const title = tab.route.name === "Chat" ? "Emberhut" : (tabTitles[tab.route.name] ?? tab.route.name);
    return (
      <Stack.Navigator
        screenOptions={{
          title,
          headerTintColor: colors.text,
          headerLargeTitle: ios && largeTitle,
          headerLargeTitleShadowVisible: false,
          ...(ios
            ? // The bar's own material: Liquid Glass's scroll-edge effect on iOS 26, the standard bar before it.
              { headerTransparent: !solid }
            : { headerStyle: { backgroundColor: colors.surface } }),
          contentStyle: { backgroundColor: colors.background },
        }}
      >
        <Stack.Screen name="screen" options={screen}>
          {(stack) => <Inner tab={tab} header={stack.navigation} />}
        </Stack.Screen>
      </Stack.Navigator>
    );
  }
  function Inner({ tab, header }: { tab: TabScreenProps<Name>; header: NativeStackNavigationProp<ParamListBase> }) {
    // The tab's navigation, with setOptions reaching the header's stack (setOptions on the tab would set tab-bar options).
    // Both objects keep their identity across renders, so the screens' header effects run once.
    const navigation: TabScreenProps<Name>["navigation"] = useMemo(
      () => ({
        ...tab.navigation,
        setOptions: (options: Partial<NativeStackNavigationOptions>) => header.setOptions(options),
      }),
      [tab.navigation, header],
    );
    return <Screen route={tab.route} navigation={navigation} />;
  }
  TabWithHeader.displayName = `WithHeader(${Screen.displayName ?? Screen.name})`;
  return TabWithHeader;
}

const tabTitles: Partial<Record<keyof TabParamList, string>> = {
  NeedsYou: "Needs you",
  Tasks: "Tasks",
  Sessions: "Sessions",
  More: "More",
};
