import type { ConfigContext, ExpoConfig } from "expo/config";
import { cidrOf, privateIpv4Ranges, privateIpv6Ranges } from "./src/api/localNetwork.ts";
import { dialect } from "./src/api/dialect.ts";
import { PUSH_CHANNEL } from "./src/api/wire.ts";
import { readAccountSettings } from "./src/config/account.ts";

// android/ and ios/ are generated from this file by `expo prebuild` (Expo CNG) and never committed.

/**
 * The Android versionCode. Bump before every Google Play upload: Play takes each one once (1 is taken), and Android never
 * installs over a higher one.
 */
const BUILD_NUMBER = 4;
/** Bump before every TestFlight upload: App Store Connect takes each build number once per version. */
const IOS_BUILD_NUMBER = 6;
const VERSION = "0.1.0";

/**
 * The app's name and domain. The name is what the home screen shows; the domain gives the bundle id and the Android
 * package (emberhut.com: com.emberhut.mobile) and the URL scheme (emberhut://).
 */
const BRAND = { name: "Emberhut", domain: "emberhut.com" };
const appId = `${BRAND.domain.split(".").reverse().join(".")}.mobile`;
const scheme = BRAND.domain.split(".")[0];

// What differs between builders comes from the environment (env.example): the Firebase file, the Apple team and, at
// Gradle time, the release keystore (plugins/withReleaseSigning.js). The server is typed in at pairing, never built in.
const googleServicesFile = process.env.GOOGLE_SERVICES_FILE;
const appleTeamId = process.env.APPLE_TEAM_ID;

/**
 * The Android channel the server's pushes name, in its spelling when the build has a wire dialect
 * (EXPO_PUBLIC_EMBERHUT_WIRE_DIALECT, src/api/dialect.ts). A malformed dialect stops the build when it's read.
 */
const pushChannel = dialect.writeText(PUSH_CHANNEL);

/**
 * The emberhut.com account's build-time settings (src/config/account.ts, ADR 0007 section 5), unset in builds from source,
 * which have no account. A half-set or malformed set stops the build here, rather than shipping an app whose account
 * fails at its first use.
 */
export function checkAccountSettings(env: Record<string, string | undefined>): void {
  const account = readAccountSettings({
    publishableKey: env.EXPO_PUBLIC_EMBERHUT_CLERK_PUBLISHABLE_KEY,
    directoryUrl: env.EXPO_PUBLIC_EMBERHUT_DIRECTORY_URL,
    passkeys: env.EXPO_PUBLIC_EMBERHUT_PASSKEYS,
  });
  if (!account.ok) throw new Error(account.error);
}
checkAccountSettings(process.env);

/** Why the camera is asked for: the in-app scanner reads pairing QR codes, and the app takes no pictures. */
export const cameraReason = "Emberhut scans the QR code that pairs this phone with your server.";
const microphoneReason = "Emberhut turns what you dictate into text.";

/**
 * iOS's App Transport Security, as narrow as reaching a self-hosted server by private IP over plain http allows: the
 * server is often reached on a home network or a VPN's CGNAT address, without a certificate. So only the loopback,
 * private (RFC 1918, RFC 4193) and CGNAT (RFC 6598) ranges are exempt, the same ranges the sign-in screen allows plain
 * http to (src/api/localNetwork.ts). Host names always need https; localhost (the Simulator) is an unqualified name,
 * which ATS doesn't block.
 *
 * What Apple's documentation says (read 25 Nov 2026):
 * - NSAllowsLocalNetworking
 *   (https://developer.apple.com/documentation/bundleresources/information-property-list/nsapptransportsecurity/nsallowslocalnetworking):
 *   iOS 10 to 16 allow connections to IP addresses by default; "In iOS 17, iPadOS 17, and macOS 14, ATS no longer allows
 *   connections to IP addresses by default. Add individual IP addresses and classless inter-domain routing (CIDR)
 *   ranges in the NSExceptionDomains dictionary." NSAllowsLocalNetworking itself enables IP addresses in general, not
 *   only private ones, so it isn't used; nor NSAllowsArbitraryLoads, which turns ATS off.
 * - NSExceptionDomains
 *   (https://developer.apple.com/documentation/bundleresources/information-property-list/nsapptransportsecurity/nsexceptiondomains):
 *   "In iOS 17, iPadOS 17, and macOS 14, you can use an IPv4 address ... or an IPv6 address ... You can also use a
 *   classless inter-domain routing (CIDR) range".
 * Not yet confirmed on a device: the iOS Release run in README.md's manual checks covers it. If the CIDR keys turn out
 * to be ignored, fall back to NSAllowsLocalNetworking, with the app's own rule still in front of it.
 */
const plainHttpExceptions = Object.fromEntries(
  [...privateIpv4Ranges.map(cidrOf), ...privateIpv6Ranges].map((range) => [range, { NSExceptionAllowsInsecureHTTPLoads: true }]),
);

export default ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name: BRAND.name,
  slug: "emberhut",
  scheme,
  version: VERSION,
  orientation: "default",
  // A placeholder until the app's own identity has an icon.
  icon: "./assets/icon.png",
  userInterfaceStyle: "automatic",
  platforms: ["android", "ios"],
  ios: {
    bundleIdentifier: appId,
    buildNumber: String(IOS_BUILD_NUMBER),
    supportsTablet: false,
    ...(appleTeamId ? { appleTeamId } : {}),
    infoPlist: {
      NSAppTransportSecurity: { NSExceptionDomains: plainHttpExceptions },
      // No encryption of its own beyond HTTPS: TestFlight needs no export compliance answer.
      ITSAppUsesNonExemptEncryption: false,
    },
  },
  android: {
    package: appId,
    versionCode: BUILD_NUMBER,
    // Android 13 on: a back swipe shows where it goes before it gets there.
    predictiveBackGestureEnabled: true,
    adaptiveIcon: { foregroundImage: "./assets/icon.png", backgroundColor: "#091828" },
    // FCM push: without the file the app builds and runs, only without push.
    ...(googleServicesFile ? { googleServicesFile } : {}),
  },
  plugins: [
    "expo-secure-store",
    // The native tab bar: Material 3's bottom navigation on Android wants a Material 3 app theme.
    ["react-native-bottom-tabs", { theme: "material3" }],
    // The server's pushes name this channel (its name on the wire); FCM uses it too for any that arrive before
    // the app made it.
    ["expo-notifications", { defaultChannel: pushChannel, color: "#F5C451" }],
    // Plain HTTP to a server on a private network, in release builds too. Android's network security config can't name
    // address ranges (only hosts), so cleartext is on for the app and the sign-in screen keeps plain http to private and
    // CGNAT addresses and localhost (src/api/localNetwork.ts); host names always need https.
    ["expo-build-properties", { android: { usesCleartextTraffic: true } }],
    // Dictation into the message box records the microphone; nothing plays or records in the background.
    ["expo-audio", { microphonePermission: microphoneReason, enableBackgroundPlayback: false }],
    // The camera only scans pairing QR codes (its barcode scanner stays in) and records no video, so it asks for no
    // microphone on Android. On iOS its microphone reason is the dictation's: a plugin given none puts in its own default.
    ["expo-camera", { cameraPermission: cameraReason, microphonePermission: microphoneReason, recordAudioAndroid: false }],
    // Pictures come from the photo picker, never the camera. Its camera reason is the scanner's all the same: `false`
    // would take the camera from the scanner too (it removes the iOS key and blocks Android's CAMERA permission).
    [
      "expo-image-picker",
      {
        cameraPermission: cameraReason,
        microphonePermission: microphoneReason,
        photosPermission: "Pictures you pick go to Emberhut with your message.",
      },
    ],
    "expo-video",
    // The camera is optional on Android (the scanner is its one use), and expo-camera's unused document scanner is out.
    "./plugins/withScannerCamera.js",
    "./plugins/withReleaseSigning.js",
  ],
});
