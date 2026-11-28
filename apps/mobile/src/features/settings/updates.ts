/**
 * Where this phone's updates come from: the store that installed it. iOS builds come through TestFlight, so the row opens
 * the TestFlight app (`itms-beta://`), or its web page when the app isn't installed; Android builds come through Google
 * Play, so it opens the app's Play listing. The phone never updates itself.
 */

/** The app's id in the stores (`app.config.ts`). */
export const appId = "com.emberhut.mobile";

/** The row that says where updates come from: its title, the address it opens, and the one to try when that fails. */
export interface StoreLink {
  title: string;
  url: string;
  fallback: string;
}

/** Where updates come from on `os`. */
export function storeLink(os: string): StoreLink {
  if (os === "ios") {
    return { title: "Updates come through TestFlight", url: "itms-beta://", fallback: "https://testflight.apple.com/" };
  }
  return {
    title: "Updates come through Google Play",
    url: `market://details?id=${appId}`,
    fallback: `https://play.google.com/store/apps/details?id=${appId}`,
  };
}

/** The version line: `0.1.0 (build 6)`, or the version alone when the build isn't known. */
export function versionLine(version: string | null | undefined, build: string | number | null | undefined): string {
  const shown = version ?? "?";
  return build === null || build === undefined || build === "" ? shown : `${shown} (build ${build})`;
}
