// The emberhut.com account is off unless the build turns it on (ADR 0007, section 5). Official builds set the Clerk publishable
// key and the directory's URL at build time; builds from source leave both unset and have no account at all: no Clerk, no
// account screen, and nothing on any screen about one. Pairing by code or by link works the same either way.
//
// Dependency-free, so app.config.ts can check the same settings when the app is built (EXPO_PUBLIC_ values are inlined
// into the bundle then, and only when read as `process.env.EXPO_PUBLIC_…` literally).

/** The account's build-time settings, when the build has it on. */
export interface AccountConfig {
  /** Clerk's publishable key: public by design, though still never in the repo. */
  publishableKey: string;
  /** Where the directory is: `https://<relay domain>`, whose API is at `/v1`. */
  directoryUrl: string;
  /** Sign in with a passkey too (Clerk's passkeys need its paid plan and the app's associated domain). */
  passkeys: boolean;
}

/** The settings as the environment gives them. */
export interface AccountEnv {
  publishableKey?: string;
  directoryUrl?: string;
  passkeys?: string;
}

export type AccountSettings = { ok: true; account: AccountConfig | null } | { ok: false; error: string };

/**
 * Reads the account's settings: both unset is off; both set (and well formed) is on; anything else is a mistake in the
 * build, which app.config.ts stops the build for.
 */
export function readAccountSettings(env: AccountEnv): AccountSettings {
  const publishableKey = env.publishableKey?.trim() ?? "";
  const directory = env.directoryUrl?.trim() ?? "";
  const passkeys = env.passkeys?.trim() ?? "";
  if (passkeys !== "" && passkeys !== "1") return { ok: false, error: "EXPO_PUBLIC_EMBERHUT_PASSKEYS is 1 to turn passkeys on, or unset." };
  if (publishableKey === "" && directory === "") {
    return passkeys === "1"
      ? { ok: false, error: "EXPO_PUBLIC_EMBERHUT_PASSKEYS needs the account on: set its publishable key and directory URL too." }
      : { ok: true, account: null };
  }
  if (publishableKey === "" || directory === "")
    return {
      ok: false,
      error: "The account needs both EXPO_PUBLIC_EMBERHUT_CLERK_PUBLISHABLE_KEY and EXPO_PUBLIC_EMBERHUT_DIRECTORY_URL, or neither.",
    };
  if (/\s/.test(publishableKey)) return { ok: false, error: "EXPO_PUBLIC_EMBERHUT_CLERK_PUBLISHABLE_KEY has white space in it." };
  const directoryUrl = directoryOrigin(directory);
  if (directoryUrl === null)
    return { ok: false, error: "EXPO_PUBLIC_EMBERHUT_DIRECTORY_URL is the directory's https origin, like https://relay.example.com." };
  return { ok: true, account: { publishableKey, directoryUrl, passkeys: passkeys === "1" } };
}

/** An https origin (a host name and an optional port, nothing after but a slash), lowercased; null for anything else. */
function directoryOrigin(text: string): string | null {
  const match = /^https:\/\/([a-z0-9](?:[a-z0-9.-]*[a-z0-9])?)(:\d{1,5})?\/?$/i.exec(text);
  return match ? `https://${match[1]!.toLowerCase()}${match[2] ?? ""}` : null;
}

const settings = readAccountSettings({
  publishableKey: process.env.EXPO_PUBLIC_EMBERHUT_CLERK_PUBLISHABLE_KEY,
  directoryUrl: process.env.EXPO_PUBLIC_EMBERHUT_DIRECTORY_URL,
  passkeys: process.env.EXPO_PUBLIC_EMBERHUT_PASSKEYS,
});

/** This build's account, or null when it has none. A build whose settings are wrong doesn't get this far (app.config.ts). */
export const accountConfig: AccountConfig | null = settings.ok ? settings.account : null;
