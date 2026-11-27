import { ClerkProvider } from "@clerk/expo";
import { tokenCache } from "@clerk/expo/token-cache";
import type { ReactNode } from "react";
import { accountConfig } from "@/config/account";

/**
 * Clerk's native passkeys, in builds that turn them on. Picked on the literal setting, as AccountProvider picks Clerk, so a
 * build without them bundles none of their code.
 */
const passkeys =
  process.env.EXPO_PUBLIC_EMBERHUT_PASSKEYS === "1"
    ? // eslint-disable-next-line @typescript-eslint/no-require-imports -- a require the bundler can drop, unlike an import
      (require("@clerk/expo/passkeys") as typeof import("@clerk/expo/passkeys")).passkeys
    : undefined;

/**
 * Clerk, for builds with the account on (src/config/account.ts; AccountProvider loads this module only then). Clerk's
 * client token is kept in the Keychain or Keystore (expo-secure-store); passkeys only when the build turns them on too.
 * Clerk's own update screen stays off: the app shows no Clerk UI.
 */
export function ClerkAccountProvider({ children }: { children: ReactNode }) {
  if (accountConfig === null) return children;
  return (
    <ClerkProvider
      publishableKey={accountConfig.publishableKey}
      tokenCache={tokenCache}
      __experimental_passkeys={passkeys}
      updateRequiredViewEnabled={false}
    >
      {children}
    </ClerkProvider>
  );
}
