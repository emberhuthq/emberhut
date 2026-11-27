import type { LinkingOptions } from "@react-navigation/native";
import * as Notifications from "expo-notifications";
import { AndroidImportance } from "expo-notifications";
import { Linking, Platform } from "react-native";
import { channelId, linkOf } from "./link";
import type { PushPlatform } from "./registration";

// expo-notifications behind PushPlatform. getDevicePushTokenAsync gives the native FCM token on Android, which is what
// the server sends to (not Expo's push service). On iOS it would give an APNs token, but the server doesn't send iPhone
// pushes to this app's bundle id yet, so this app doesn't ask for any there.

// One of the server's alerts while the app is open still shows: the server already holds those for the room on screen.
Notifications.setNotificationHandler({
  handleNotification: async () => ({ shouldShowBanner: true, shouldShowList: true, shouldPlaySound: true, shouldSetBadge: false }),
});

export const expoPush: PushPlatform = {
  unsupported: Platform.OS === "ios" ? "Not on iPhone yet: the server doesn't send iPhone notifications to this app" : undefined,
  async prepare() {
    await Notifications.setNotificationChannelAsync(channelId, {
      name: "Emberhut",
      description: "Emberhut's messages and what needs you",
      importance: AndroidImportance.HIGH,
    });
  },
  async permission() {
    const { status, canAskAgain } = await Notifications.getPermissionsAsync();
    return { status, canAskAgain };
  },
  async requestPermission() {
    return (await Notifications.requestPermissionsAsync()).granted;
  },
  async token() {
    return String((await Notifications.getDevicePushTokenAsync()).data);
  },
  onTokenChange(listener) {
    const subscription = Notifications.addPushTokenListener((token) => listener(String(token.data)));
    return () => subscription.remove();
  },
  async forget() {
    // On Android this deletes the FCM token.
    await Notifications.unregisterForNotificationsAsync();
  },
};

/** The link a tapped notification carries: in its content's data, or in FCM's own message when Android showed it. */
function responseLink(response: Notifications.NotificationResponse | null): string | null {
  if (!response) return null;
  const { content, trigger } = response.notification.request;
  const remote = trigger && "remoteMessage" in trigger ? trigger.remoteMessage?.data : undefined;
  return linkOf(content.data) ?? linkOf(remote);
}

/** React Navigation's links, with the ones a tapped notification opens (cold start and while running). */
export const notificationLinking: Pick<LinkingOptions<object>, "getInitialURL" | "subscribe"> = {
  async getInitialURL() {
    return (await Linking.getInitialURL()) ?? responseLink(Notifications.getLastNotificationResponse());
  },
  subscribe(listener) {
    const links = Linking.addEventListener("url", ({ url }) => listener(url));
    const taps = Notifications.addNotificationResponseReceivedListener((response) => {
      const link = responseLink(response);
      if (link) listener(link);
    });
    return () => {
      links.remove();
      taps.remove();
    };
  },
};
