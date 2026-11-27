import type { LinkingOptions } from "@react-navigation/native";
import * as Application from "expo-application";
import * as Device from "expo-device";
import * as Notifications from "expo-notifications";
import { AndroidImportance } from "expo-notifications";
import { Linking, Platform } from "react-native";
import { channelId, tappedLink } from "./link";
import { apnsEnv, type PushPlatform } from "./registration";

// expo-notifications behind PushPlatform. getDevicePushTokenAsync gives the native token, which is what the server sends
// to (not Expo's push service): FCM's on Android, the APNs device token (hex) on an iPhone.

// One of the server's alerts while the app is open still shows: the server already holds those for the room on screen.
Notifications.setNotificationHandler({
  handleNotification: async () => ({ shouldShowBanner: true, shouldShowList: true, shouldPlaySound: true, shouldSetBadge: false }),
});

const ios = Platform.OS === "ios";

/** The aps-environment of the profile this build was signed with, or null without one (or when it can't be read). */
async function apsEnvironment(): Promise<string | null> {
  try {
    return await Application.getIosPushNotificationServiceEnvironmentAsync();
  } catch {
    return null;
  }
}

export const expoPush: PushPlatform = {
  async prepare() {
    if (ios) return;
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
    return (await Notifications.requestPermissionsAsync({ ios: { allowAlert: true, allowBadge: true, allowSound: true } })).granted;
  },
  async token() {
    return String((await Notifications.getDevicePushTokenAsync()).data);
  },
  async address(token) {
    if (!ios) return { token, platform: "android" };
    return { token, env: apnsEnv(await apsEnvironment(), Device.isDevice) };
  },
  onTokenChange(listener) {
    const subscription = Notifications.addPushTokenListener((token) => listener(String(token.data)));
    return () => subscription.remove();
  },
  async forget() {
    // On Android this deletes the FCM token. iOS keeps its APNs token (Apple advises against unregistering), and the
    // server forgets it with the sign-out.
    if (!ios) await Notifications.unregisterForNotificationsAsync();
  },
};

/** The link a tapped notification carries, wherever its platform put the server's data (push/link.ts). */
function responseLink(response: Notifications.NotificationResponse | null): string | null {
  if (!response) return null;
  const { content, trigger } = response.notification.request;
  return tappedLink(content.data, trigger);
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
