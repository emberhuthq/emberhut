import type { ReactNode } from "react";

type Provider = (props: { children: ReactNode }) => ReactNode;

/**
 * The account's provider: Clerk in builds with the account on, nothing in the others. It is chosen on the literal
 * `process.env.EXPO_PUBLIC_…`, which Metro inlines when it bundles, so in a build without the account the branch that
 * requires Clerk is dead code and Clerk's packages are left out of the bundle altogether (CI checks that they are).
 */
export const AccountProvider: Provider = process.env.EXPO_PUBLIC_EMBERHUT_CLERK_PUBLISHABLE_KEY
  ? // eslint-disable-next-line @typescript-eslint/no-require-imports -- a require the bundler can drop, unlike an import
    (require("./ClerkProvider") as typeof import("./ClerkProvider")).ClerkAccountProvider
  : ({ children }) => children;
