import { NavigationContainer } from "@react-navigation/native";
import { QueryClientProvider } from "@tanstack/react-query";
import { StatusBar } from "expo-status-bar";
import { useMemo } from "react";
import { AppState } from "react-native";
import { KeyboardProvider } from "react-native-keyboard-controller";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { useStore } from "zustand";
import { AccountProvider } from "@/account/AccountProvider";
import { UndoToast } from "@/features/needsYou/UndoToast";
import { inBackground } from "@/log";
import { RootStack, linking } from "@/navigation/RootStack";
import { navigationTheme } from "@/navigation/theme";
import { actionsStore, pushStore, sessionStore } from "@/services";
import { followAppFocus } from "@/state/appFocus";
import { usePrefs, useSession } from "@/state/hooks";
import { queryClient } from "@/state/query";
import { ThemeProvider, useTheme } from "@/theme";
import { builtInTheme } from "@/theme/builtIn";
import { reduceTransparencyStore } from "@/ui";

followAppFocus();
inBackground(sessionStore.getState().start(), "App: start");
// Leaving the app sends a held action at once instead of after its undo window; coming back registers push if it wasn't.
AppState.addEventListener("change", (state) => {
  if (state !== "active") actionsStore.getState().flush();
  else if (!pushStore.getState().registered) inBackground(pushStore.getState().sync(), "App: sync");
});

function Navigation() {
  const theme = useTheme();
  const navTheme = useMemo(() => navigationTheme(theme), [theme]);
  return (
    <>
      <StatusBar style={theme.scheme === "dark" ? "light" : "dark"} />
      <NavigationContainer theme={navTheme} linking={linking}>
        <RootStack />
      </NavigationContainer>
      <UndoToast />
    </>
  );
}

export default function App() {
  const appearance = usePrefs((s) => s.themeCache?.choice.appearance ?? s.theme);
  const tokens = usePrefs((s) => s.themeCache?.tokens ?? builtInTheme.tokens);
  // Until secure storage is read, nothing knows yet whether to show sign-in or the app; until Reduce Transparency is,
  // whether its bars and message box may be see-through.
  const starting = useSession((s) => s.status === "starting");
  const transparencyKnown = useStore(reduceTransparencyStore, (s) => s.known);
  return (
    <SafeAreaProvider>
      <KeyboardProvider>
        <QueryClientProvider client={queryClient}>
          <AccountProvider>
            <ThemeProvider appearance={appearance} tokens={tokens}>
              {starting || !transparencyKnown ? null : <Navigation />}
            </ThemeProvider>
          </AccountProvider>
        </QueryClientProvider>
      </KeyboardProvider>
    </SafeAreaProvider>
  );
}
