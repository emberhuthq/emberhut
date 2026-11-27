import AsyncStorage from "@react-native-async-storage/async-storage";
import * as Crypto from "expo-crypto";
import * as Device from "expo-device";
import { File, UploadType } from "expo-file-system";
import { fetch } from "expo/fetch";
import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";
import {
  AuthSession,
  createApi,
  createReach,
  createServerAddress,
  EmberhutHttp,
  type Fetch,
  type SecureStorage,
  type UploadFile,
} from "@/api";
import { createDirectory, type Directory } from "@/account/directory";
import { accountConfig } from "@/config/account";
import { inBackground, logError } from "@/log";
import { expoPush } from "@/push/native";
import { createPushStore } from "@/push/registration";
import { createActionsStore } from "@/state/actions";
import { createChatStore } from "@/state/chat";
import { createConnectionStore } from "@/state/connection";
import { createLiveStore } from "@/state/live";
import { createPrefsStore } from "@/state/prefs";
import { queryClient, queryKeys } from "@/state/query";
import { createSessionStore } from "@/state/session";
import { createTalkStore } from "@/state/talk";
import { createThemeStore } from "@/state/theme";

// The app's one set of services, built from the native pieces. Tests build their own from the same factories.

const storage: SecureStorage = {
  getItem: (key) => SecureStore.getItemAsync(key),
  setItem: (key, value) => SecureStore.setItemAsync(key, value),
  deleteItem: (key) => SecureStore.deleteItemAsync(key),
};

// The server is typed in at pairing and kept here. Until then the app shows sign-in (Not now needs a server) and asks
// nothing; the reserved .invalid name only makes a stray request fail at once.
export const serverAddress = createServerAddress(storage);
const baseUrl = () => serverAddress.url ?? "http://server.invalid";
// expo/fetch has its own Request and Response types; at runtime it answers what the API layer reads of a DOM Response.
const nativeFetch = fetch as unknown as Fetch;

// The file goes up from disk as it is read, never whole in memory; the reply comes back as a Response like fetch's.
const uploadFile: UploadFile = async (fileUri, url, { headers, signal }) => {
  const result = await new File(fileUri).upload(url, { httpMethod: "POST", uploadType: UploadType.BINARY_CONTENT, headers, signal });
  return new Response(result.body, { status: result.status, headers: result.headers });
};

// What the server calls this phone when the system won't say.
const phoneKind = Platform.OS === "ios" ? "iPhone" : "Android";

const auth = new AuthSession({ baseUrl, fetch: nativeFetch, storage, newId: Crypto.randomUUID, log: logError });
export const api = createApi(new EmberhutHttp({ baseUrl, fetch: nativeFetch, auth, uploadFile }));

export const sessionStore = createSessionStore({
  auth,
  api,
  address: serverAddress,
  device: { name: Device.deviceName ?? Device.modelName ?? phoneKind, model: Device.modelName ?? phoneKind },
  reach: createReach(nativeFetch, (size) => Crypto.getRandomBytes(size)),
});
export const prefsStore = createPrefsStore(AsyncStorage);
export const themeStore = createThemeStore(api, prefsStore);
export const chatStore = createChatStore(api);
export const liveStore = createLiveStore();
export const connectionStore = createConnectionStore();
export const pushStore = createPushStore({
  platform: expoPush,
  register: (token) => api.devices.registerPush(token),
  session: sessionStore,
});
export const talkStore = createTalkStore({
  api,
  chat: (room) => chatStore.getState().chats[room] ?? [],
  onSent: (room) => inBackground(queryClient.invalidateQueries({ queryKey: queryKeys.state(room) }), "services: invalidateQueries"),
});
export const actionsStore = createActionsStore(api, {
  onApplied: () => inBackground(queryClient.invalidateQueries({ queryKey: queryKeys.allStates }), "services: invalidateQueries"),
});

// What the server held for a session that left belongs to nobody now.
sessionStore.subscribe((now, before) => {
  if (before.status === "signedIn" && now.status === "signedOut") queryClient.clear();
});

/** The account directory's client, calling with the account's token; null in a build without the account. */
export function accountDirectory(token: () => Promise<string | null>): Directory | null {
  return accountConfig === null ? null : createDirectory({ baseUrl: accountConfig.directoryUrl, fetch: nativeFetch, token });
}
