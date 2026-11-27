import { useNavigation } from "@react-navigation/native";
import { useCallback } from "react";
import { Linking } from "react-native";
import { emberhutLink } from "./emberhutLink";

/** Opens a link from a message: Emberhut's own in the app, anything else in its app or the browser. */
export function useOpenLink(): (url: string) => void {
  const navigation = useNavigation();
  return useCallback(
    (url: string) => {
      const link = emberhutLink(url);
      if (link === null) {
        void Linking.openURL(url).catch(() => undefined);
        return;
      }
      if (link.screen === "Browser" || link.screen === "Parties" || link.screen === "Approvals") navigation.navigate(link.screen);
      else if (link.screen === "Session") navigation.navigate("Session", { id: link.id });
      else navigation.navigate(link.screen, { id: link.id });
    },
    [navigation],
  );
}
