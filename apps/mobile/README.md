# apps/mobile

Emberhut for iPhone and Android: one React Native codebase on Expo, in TypeScript. It talks to an Emberhut server over its HTTP
API (the chat and its rooms, agents and their approvals, Tasks, Needs you, sessions, the live browser, About you,
settings).

Expo SDK 58, React Native 0.88, React 19.3, React Navigation 7 with react-native-bottom-tabs (native tab bars) and a native
stack per tab, zustand for app state and
TanStack Query for server state. Package `@emberhut/mobile`, app name "Emberhut", bundle id and Android package `com.emberhut.mobile`.
The name and domain are one line, `BRAND` in `app.config.ts`.

The few values the API contract spells (the owner's and the assistant's names in task owners and replies, the header that
says which room is on screen, the Android push channel, the app's link scheme `emberhut://` and the task-key prefix) are
each written once, in `src/api/wire.ts`, and read from there. Moving the API to `packages/api-client`, generated from
`crates/emberhut-server/openapi.json`, is the next step ([docs/architecture.md](../../docs/architecture.md)).

## Setup

- Node 22 and the pnpm pinned in the root `package.json`: `pnpm install --frozen-lockfile` at the repository root.
- iOS: a Mac with Xcode 26 and CocoaPods.
- Android: JDK 17 or newer, the Android SDK with `ANDROID_HOME` set (platform 36, build-tools 36, NDK 27).
- `cp env.example .env` here, then fill in what you need (each line says what it's for). No secret goes into the repo,
  the Expo config or the bundle: the Firebase file and the keystore stay outside and only their paths are configured.

## First run

1. A server, and a way for the phone to reach it (the same network, or the VPN it sits behind).
2. `pnpm run ios` (Simulator) or `pnpm run android` (a phone on USB or an emulator): a debug build and Metro.
3. Pair: scan the pairing QR code (or paste the pairing link) that a device already signed in shows (Settings › Devices,
   Add a phone or computer) or the server's pairing command prints; or type the server's address (`https://my-server:18990`;
   `https://` is assumed) and the one-time code. A code works once, for 10 minutes. The phone keeps the address, its
   refresh token and the machine's key in secure storage (the iOS Keychain, the Android Keystore).

## Pairing by link, and the machine's key

The pairing link is `https://emberhut.com/pair#id=…&code=…&route=…` ([docs/api.md](../../docs/api.md), "The pairing link").
The app reads it with its own scanner (expo-camera, which asks for the camera only then), never through the system, so
no other app is handed the code. On Android the camera is declared optional (`plugins/withScannerCamera.js`), so the app
installs on a device without one, and expo-camera's document scanner is turned off. Its barcode scanner bundles ML Kit's
barcode model, which ML Kit's documentation puts at about 2.4 MB of app size; keep it in mind against Google
Play's upload limit. `src/connect/` reads the link, the machine id and the encodings by the same rules as
`crates/emberhut-identity`, and its tests run the shared vectors in `crates/emberhut-api/tests/fixtures/connect/`.

- **From a link**, the phone probes every route in it at once (2.5 s each) and sends the code only to the first, in the
  link's order, where the machine proved itself: `GET /api/connect/probe` with a fresh nonce, answered with the link's
  machine id, a key that gives it, an Ed25519 signature over the `emberhut probe v1` statement (checked by the contract's one
  rule, `src/connect/signature.ts`: canonical points of the prime-order subgroup, the cofactorless equation) and the
  route's own address or name (`src/connect/probe.ts`). An address is probed as an address route, a name as a name
  route, which must be https.
- **A server older than the probe** answers it 404 or 405. Only then is `GET /api/auth/status` asked, and the route
  counts only for a 200 of exactly that answer's shape; the pairing answer's key is then the one proof. A 401, a portal's
  page or anything else is never Emberhut. This fallback is removed once servers serve the probe.
- Routes go by the typed address's rule below (https, or plain http only to a private address); a relay route needs a
  sealed session, which the app doesn't open yet, so it is skipped.
- **The answer's machine key** must give its id (the hash of the key, `src/connect/machineId.ts`), and that id must be
  the link's. Otherwise: "This server didn't prove it is the machine you meant", and none of its tokens are kept.
- **The kept address** changes only when a pairing is accepted, and before its tokens are held: a wrong code or another
  machine's answer leaves the server the phone had.
- **Typed** pairing trusts the first key it sees and keeps it with the address. Pairing at that address again expects
  the same machine, and an answer without a key no longer passes. Settings › Advanced › Diagnostics shows the id, to compare
  with the one the server prints. A server older than machine keys pairs as before, only on a first, typed pairing.

## The fallback server

The phone can keep a second server beside its own, with its own pairing: the fallback server, which the app goes to by
itself when its server stops answering (`src/state/fallback.ts`).

- **Health**: while the app is in front and on its server, `GET <server>/health` (no session) every 15 s; a 2xx within
  4 s is an answer. After 3 failures in a row, every request goes to the fallback server with its session, and the chat
  says so ("… isn't answering. You're on the fallback server") with Try again. At launch the server is checked at once,
  and the app falls back when it doesn't answer after 2 quick retries. A launch always starts on the server.
- **Going back** happens only when you tap Try again (the banner, or Settings) and the server answers. Never by itself: a
  server that answers once after an outage may not stay up, and a switch under you mid-task loses less going one way.
- **Setting it up**, Settings, Fallback server, both by pairing (a pairing link, scanned or pasted, or an address and a
  code): _Move to a new server_ pairs with the new one and keeps the one the phone had, with its session as it is, as the
  fallback; _Add a fallback server_ pairs one beside the server. Remove signs out of it. Signing out of Emberhut signs
  out of both.
- **Secure storage**: the fallback's address and session sit beside the server's, under `emberhut.fallback.*`
  (`slotPrefix`, `src/api/server.ts`); the install id is one for both.
- **The wire dialect** (next section) is spoken only to a server marked as speaking it. A server is marked so by
  default, so a dialect build keeps talking to the server it was paired with as before; a new server paired with _Move to
  a new server_ isn't, and the one kept as the fallback keeps its mark. In a dialect build, Settings has a switch for
  each. A build without a dialect has nothing to rewrite either way.

## Building the app for a server that spells some wire words differently

A build can carry a wire dialect: `EXPO_PUBLIC_EMBERHUT_WIRE_DIALECT` (see `env.example`) holds its JSON, and
`src/api/dialect.ts` reads it once. Unset or empty, the app speaks its own words and nothing is rewritten. With one, the
API client reads every answer (JSON bodies, error bodies, the live streams' events) into the app's words and writes
every request (JSON bodies, query strings, header names) back into the server's; a push's link is read the same way, and
`app.config.ts` creates the Android push channel under the server's name. The app's code only ever sees the words in
`src/api/wire.ts`.

```json
{
  "words": [["person", "owner"]],
  "keys": [["helperAccount", "assistantAccount"]],
  "headers": [["X-Other-Looking", "X-Emberhut-Looking"]],
  "prefixes": [
    ["K-", "T-"],
    ["other://", "emberhut://"]
  ]
}
```

Each pair is the server's spelling, then the app's. The rules are the same in the desktop client (`emberhut-client`'s
`dialect` module) and the browser extension, and `crates/emberhut-client/tests/fixtures/dialect.json` holds the cases all
three must answer alike:

- `words`: a string value that is exactly one of them is replaced whole, both ways. Reading an answer, an object key that
  is one is renamed too (a map keyed by who), after `keys`; writing a request, field names and query names are renamed by
  `keys` only, since the app's words are field names too (`owner`).
- `keys`: a field's name or a query parameter's name, both ways; reading, looked up before `words`.
- `headers`: a header's name, in any case.
- `prefixes`: text that starts a token inside a string value that isn't a whole word (a task key, a link, a title):
  - a token starts at the start of the string or right after a character that can't be part of one; token characters
    are letters, digits and `_ - = + / : .` So `(K-1,K-2)`, `"K-1"` and `[it](other://x)` are rewritten, and `OK-1`,
    `id=K-1`, `a/K-1` and `a.K-1` aren't: tokens, ids and addresses aren't rewritten inside. A link scheme (a prefix
    ending in `://`) keeps the same rule, so `?next=other://x` stays as it is;
  - a prefix ending in `-` (a key's prefix) matches only when a digit (`0`-`9`) follows it: `K-12` is rewritten, `K-shirt`
    and a lone `K-` aren't, either way;
  - where several match at one place the longest wins (the first listed among equals);
  - the text is read left to right as given: a replacement is never searched again, and where a token starts is decided
    by the character before it in the given text.

That includes what you type, so with the example above "T-4" goes to the server as "K-4" (and "a T-shirt" as it is).
`jsonFromServer`, `jsonToServer`, `fromServer` and `toServer` (`src/api`) apply this build's dialect to a JSON document
or one string the app passes to or takes from the server some other way. A malformed dialect stops `app.config.ts`.
Metro inlines the variable when it bundles, so restart it (`expo start --clear`) after changing it.

### Showing task keys with another prefix

`EXPO_PUBLIC_EMBERHUT_TASK_KEY_PREFIX` (letters and digits ending in a dash, e.g. `WK-`) is how the build shows task
keys: in messages, chips, rows and searches, task 12 reads `WK-12` instead of the contract's `T-12`. Every server's
answers are read into it and requests written back in the server's own spelling. A server that doesn't speak the
build's dialect is spoken to in the contract's words with `T-` read as the build's prefix both ways. With a dialect, a
`prefixes` pair whose app side is `T-` now ends in the build's prefix, and a pair left the same on both sides is
dropped, so a server that already spells keys that way keeps them as they are. Unset or empty, keys read `T-`.

## The emberhut.com account

Off unless the build turns it on (ADR 0007, section 5). It finds your machines; it never lets anyone into one.

- **Build-time settings** (`env.example`, read once in `src/config/account.ts`): `EXPO_PUBLIC_EMBERHUT_CLERK_PUBLISHABLE_KEY`
  and `EXPO_PUBLIC_EMBERHUT_DIRECTORY_URL` (the directory's origin, `https://<relay domain>`), both or neither: one alone, or a
  directory that isn't an https origin, stops `app.config.ts`. `EXPO_PUBLIC_EMBERHUT_PASSKEYS=1` adds sign-in with a passkey.
  Metro inlines them into the bundle when it builds, so restart it (`expo start --clear`) after changing them.
- **Self-hosters and builds from source leave them empty.** Then no screen mentions an account, and none of Clerk is in
  the app: `src/account/AccountProvider.tsx` and `RootStack.tsx` require Clerk's modules on the literal
  `process.env.EXPO_PUBLIC_…`, which Metro inlines, so without the settings that code is dead and left out of the bundle
  (4.7 MB of Hermes bytecode on Android, against 6.6 MB with the account). CI exports the Android bundle with the
  settings unset and fails if Clerk's provider or any `@clerk/` package is in it. Pairing by code and by link works the same.
- **What Clerk brings into official builds**: `@clerk/expo` and its `@clerk/clerk-js` tree, whose JavaScript holds code
  for features the app doesn't use (web3 wallets through `@coinbase/wallet-sdk` and `@base-org/account`, Stripe, password
  strength), with `crypto-js` (deprecated upstream, still Clerk's) and `core-js`. `@clerk/react` imports `react-dom`, so
  the app depends on it: keep `react-dom` at exactly the version of `react` (both pinned) when either moves.
- **On**: More, Account. Sign in with a code Clerk emails (the app's own screens, `@clerk/expo`; Clerk's client token in
  secure storage through its expo-secure-store cache). The account's machines come from `GET /v1/machines`
  ([docs/directory.md](../../docs/directory.md)) with `getToken({ template: "emberhut-directory" })`, with whether each is
  connected or when it was last seen, polled every 30 s while on screen; Unlink is `DELETE /v1/machines/{id}`. A 401
  signs the phone out of the account (never out of a machine); 429 and 503 only say to try again later.
- **Tapping a machine** starts the normal pairing with that machine pinned: the answer's key must give its id. A phone
  paired with another server signs out of it first; the sign-in screen then says which machine it's pairing with.
- **Passkeys** (`@clerk/expo-passkeys`, given the `buffer` package it loads without declaring it by a `packageExtensions`
  entry in `pnpm-workspace.yaml`) are required only when `EXPO_PUBLIC_EMBERHUT_PASSKEYS=1`, so their JavaScript is bundled
  only then. Their native module is still linked into every build: Expo's autolinking takes exclusions only from
  `package.json`, not per build. They need Clerk's paid plan, and the app's associated domain (`webcredentials:`) set up
  with the store builds; until then they stay off. Signing in with Google isn't offered: Clerk's browser flow returns
  through a custom URL scheme, which ADR 0007 rules out for account codes, and its native Google sign-in needs another
  native module and Google client ids.

Plain HTTP is allowed only to localhost and to IP addresses in the private, loopback and CGNAT ranges
(`src/api/localNetwork.ts`), since a self-hosted server is often reached that way on a home network or a VPN; it is the
default there, and the sign-in screen marks it as not encrypted. A server by name always needs https. On iOS, App
Transport Security exempts only those ranges (`NSExceptionDomains` takes CIDR ranges since iOS 17, which stopped letting
plain http reach IP literals, as Apple's documentation cited in `app.config.ts` says), not the whole app
(`NSAllowsArbitraryLoads`). Android's network security config can't name address ranges, so cleartext is on for the app
and the sign-in screen enforces the rule.

The live browser view follows the same rule: the stream address the server sends (`GET /api/browser/session`) must be
https, or plain http to a private or CGNAT IP. On iOS anything else is blocked by ATS; a page that fails to load says
why, with Retry.

## Tasks with plans

A server that keeps task plans says so in the state (`ui.taskPlans`). Then every task carries the plan its agent ticks
as it goes: the state's tasks have it at a glance (`plan`: done of total, the current step, what blocks it, the
estimate, when it changed), and a task's page has every step (`GET /api/tasks/<id>`'s `fullPlan`; the task's own `plan`
there stays the glance). The step states are wire words in `src/api/wire.ts`; the reading is `features/tasks/plan.ts`
and `planView.ts`, with their tests. Without the switch (an older server) nothing below happens and the app is as it
was.

- **Tabs**: Your ember's, Needs you, Tasks and More. Sessions leaves the tab bar and opens as a page from More (and from the
  chat's Sessions button). The switch's last value is kept with the prefs, so the bar is right before the state answers.
- **Tasks**: a line to what needs you (its tab lists it), then Running (a blocked task that runs stays here with its red
  mark; then the sessions running with no task), Blocked, Open, and the other sessions with no task (waiting on you, or
  finished in the last day), each opening its session. A row leads with a shape for its state (a turning ring while a run
  goes, red when blocked, the ember dot when it waits on you), keeps its title whole, says the step it's on or what blocks
  it, "quiet 25m" once a running task's plan and session haven't moved for 20 minutes, and ends on a bar with a segment per
  step (plain past 12) and its ticks. Running is the theme's warn colour, as on the desktop, always with a shape.
- **A task's page**: under the title its state, Now, what needs you or blocks it (a tap scrolls to the ask), Next, the
  estimate and its session; the count ("4 of 8 done · 1 blocked · 1 skipped") over the segmented bar; the checklist (the
  current step lit with its time so far, sub-steps indented, a step's result under it, folded around the current step past
  eight rows); the ask; your reply; Files and results, each step's first; one Activity feed of turns, history and files
  (All, Turns, Plan, Files) with each turn's session one tap away. An empty plan reads "No plan yet".
- **Move to task…**: a session's screen has it under its tasks, a task page's session line has a visible Move… beside Open
  session, and a long press on a session (the Sessions list, a session with no task in Tasks, a task page's session
  line, its Activity and Sessions) offers it, also as an accessibility action. It lists the open tasks, found by key or title, and moves the runner's or worker's session to the one picked
  (`POST /api/runner/sessions/<id>/task`, `{ task }`; the route's word is in `src/api/wire.ts`), then opens that task; a
  refusal (a done or archived task, an unknown session or task) shows the server's reason. Sessions with no task shows
  only while one has none.
- **Rename**: a task page's header menu (⋯) has Rename, which opens the title in a field; Save sends it, trimmed
  (`PATCH /api/tasks/<id>`, `{ title }`; the path's start is `TASK_PATH` in `src/api/wire.ts`), then the page and the
  lists say the new title. A refusal (an empty or too long title, an unknown task) shows the server's reason.
- **Live**: the list follows the state's poll; an open page is fetched again when the state says its task, its plan or its
  run moved.

## Manual checks before a store upload

Not run by CI (no Xcode or Android SDK there); record the result in the pull request.

- Pairing by QR code, on a device (the Simulator has no camera): the camera is asked for only when Scan is tapped; a QR
  code from Pair another device or the server pairs, a QR code that isn't a pairing link is said so, and a link whose
  machine answers with another key is refused. With the account on: sign in by emailed code, the machines listed, Unlink,
  sign out, and a tapped machine pinned on the sign-in screen.
- iOS, a Release build on iOS 26 (Simulator or device), on a Mac:
  - pairing over `http://` to a private (192.168/16) and a CGNAT (100.64/10) address works;
  - an `http://` address by name is refused by the sign-in screen;
  - the live browser view opens with the address the server sends.
- Android, a debug build on an emulator or phone with API 35 or later (edge to edge, where the window doesn't resize for
  the keyboard): the text box rises with the keyboard and stays above it in the chat (with the list dragged down to put
  the keyboard away), on the sign-in screen, in New session (a modal) and in a task's reply box. In the chat, no message
  shows through the gaps around the message box (it's solid there, as on iOS with Reduce Transparency on), and the tab
  bar stays under the keyboard instead of riding on top of it. The rooms sheet is solid over a dimmed chat, scrolls,
  and a tap above it closes it. The chat's title is centred, and the Sessions button's count sits whole on its icon's
  corner.
- The native chrome, on iOS 26 and on one older iOS (18):
  - the five tabs (four with `ui.taskPlans` on: Sessions moves under More): four with SF Symbols, Your ember's with the
    ember in its own colours (grey off duty), and the Needs you badge;
  - large titles that collapse as Needs you, Tasks, Sessions and More scroll;
  - the chat's solid header with no scroll-edge fade over its list, on a cold start straight into the chat (iOS 26
    Simulator: quit the app, open it), and again after switching rooms and tabs;
  - VoiceOver on the tab bar: each tab's label, and the badge read out.
- The native chrome, on Android 14 or later:
  - predictive back from a tab's root, from a pushed screen, from a modal, and from New session's folder browser (which
    holds back for itself);
  - the Material 3 bar with all five labels, the ember untinted on Your ember's tab, and the pill behind the tab picked;
  - TalkBack on the tabs;
  - dialogs (`Alert`) under the Material 3 theme.

## iOS

- `pnpm run ios:prebuild` generates `ios/` from `app.config.ts` (it is never committed), then `cd ios && pod install`.
- Simulator builds need at least ad hoc signing (`CODE_SIGN_IDENTITY=-`): without it the app has no Keychain access and
  pairing can't keep its tokens.
- Push: after pairing the app asks for notifications (alerts, badges, sounds) and gives the server its APNs device token
  with the service it belongs to (`env`). That comes from the aps-environment of the provisioning profile the build was
  signed with: a development build's profile says `development` (the sandbox); TestFlight and App Store builds carry no
  profile, and ad hoc ones say `production` (production). The server's APNs key must be for the bundle id
  `com.emberhut.mobile`.
- TestFlight: bump `IOS_BUILD_NUMBER` in `app.config.ts`, prebuild, archive the `Emberhut` scheme with an App Store profile
  for `com.emberhut.mobile` (`APPLE_TEAM_ID` in `.env`), and upload with an App Store Connect API key.

## Android

- `pnpm run android:prebuild` generates `android/` (never committed); `pnpm run android:release` builds a release APK
  with the bundle inside: `android/app/build/outputs/apk/release/app-release.apk`.
- Google Play: `pnpm run android:bundle` makes the signed AAB (`android/app/build/outputs/bundle/release/app-release.aab`),
  signed with the upload key (it stops without one); Play App Signing re-signs it for phones. It leaves out 32-bit x86,
  which no phone runs, to stay under the console's upload size. Bump `BUILD_NUMBER` (the versionCode) in
  `app.config.ts` before every upload: Play takes each one once.
- Push: a Firebase project with an Android app for `com.emberhut.mobile`, its `google-services.json` at
  `GOOGLE_SERVICES_FILE`, then build again.
- Release signing: a keystore and a properties file beside it (`storeFile`, `storePassword`, `keyAlias`, `keyPassword`),
  both outside the repo, at `EMBERHUT_ANDROID_KEYSTORE_PROPERTIES`. Without it, release builds use the debug key. Keep the
  keystore safe: Android only installs an update signed with the same key. The key, its passwords and its path never
  go into the repo.

## Commands

| Command                                      | What                                                                      |
| -------------------------------------------- | ------------------------------------------------------------------------- |
| `pnpm start`                                 | Metro, for a debug build                                                  |
| `pnpm run ios` / `pnpm run android`          | A debug build on the Simulator or a device, with Metro                    |
| `pnpm run ios:prebuild` / `android:prebuild` | Generate `ios/` or `android/` from `app.config.ts`                        |
| `pnpm run android:release`                   | A release APK                                                             |
| `pnpm run android:bundle`                    | The signed AAB for Google Play                                            |
| `pnpm run typecheck`                         | `tsc` in strict mode                                                      |
| `pnpm run lint`                              | ESLint (Expo's flat config, no `any`, no floating promises), 0 warnings   |
| `pnpm run format` / `format:check`           | Prettier                                                                  |
| `pnpm test`                                  | Vitest: the API layer, the stores, push, formats and each feature's logic |
| `pnpm run check`                             | All four, as CI runs them                                                 |

## Architecture

```
App.tsx              providers: safe area, the query cache, the theme, navigation
app.config.ts        the Expo config; android/ and ios/ are generated from it (Expo CNG)
assets/              the icon, Emberhut's face (and its grey off-duty copy), and Android's tab icons (assets/tabs)
plugins/             config plugins: release signing for the generated Gradle project, the camera as the scanner uses it
src/
  config/            Main's room id, the app's link schemes, and whether the build has the account (account.ts)
  connect/           the connect contract's formats: the pairing link, the machine id, its encodings
  account/           the emberhut.com account: Clerk's provider (only when on) and the directory's client
  api/               HTTP client, the server address, session, live streams, endpoints, response types
    types/           what the server returns, one file per domain
    endpoints/       one module per group of routes
  state/             zustand stores, the query cache, query keys and hooks
  services.ts        the app's one api, session and stores, built from the native pieces
  navigation/        the root stack, the native tabs and each tab's native header, their params, deep links
  theme/             the palette from the theme's tokens, the built-in default, spacing, type, the theme provider
  push/              push (FCM, APNs): the permission, the token given to the server, the link a tapped notification opens
  format/            dates and times as the app says them ("yesterday", "3d late", "Mon 14:05"), sizes ("12.3 GB")
  ui/                shared primitives: Screen, Text, Button, Card, ListRow, Section, SectionHeading, Chip,
                     Collapsible, FoldRow, ProgressBar, Segmented, TextField, Toggle, ActionSheet, Markdown, Face,
                     EmptyState, ErrorState, Loading, Icon, Glass (iOS Liquid Glass), KeyboardAvoider
  features/<name>/   one folder per feature: its screens, and its logic in plain modules with their tests
  test/              fakes for the tests (a stand-in server, memory storage)
```

**Data flow.** Server data goes through TanStack Query: `useEmberhutState(room)` polls `GET /api/state` every 3 s while the
app is in front (every second while failing, and "Can't reach Emberhut" only after 5 s of failures). The
state carries an ETag: the next poll sends it back as `If-None-Match`, and a 304 returns the same object, so nothing
re-renders. The looking header (`LOOKING_HEADER` in `src/api/wire.ts`), set to the room, rides along while the app is in front, so the server holds that room's pushes.
Smaller pages (`useTask`, `useTasks`, `useFacts`, `useSwitches`, `useEnvironments`, `useMachines`, `useParties`,
`useApprovals`, `useApproval`, `useAgents`, all in `state/queries.ts`) have their own keys in `state/query.ts`. The Machines page (More, Machines) polls `GET /api/machines`
every 10 s only while it's on screen; a 404 means the server is on older code and isn't asked again. Changes go through `useApplySettings()` (`POST /api/settings`), which refreshes the
states after.

App state lives in zustand stores, each made by a factory that takes its dependencies (so the tests build their own):

- `session`: signed in or not, whether the server requires it, whether this phone is refused (not on the server's network), the server's address, pairing.
- `chat`: each room's messages as the phone holds them, joined page by page (the state's newest 60, catch-up after the last
  one held, "Show earlier").
- `live`: each room's live line from its stream.
- `connection`: whether polls are failing, and since when.
- `actions`: a tap on a task (an answer, Done, Not now, Archive) hides it at once and is sent after a 4 s undo window.
- `prefs`: the room on screen and the Needs you grouping, kept across launches; also the last `GET /api/theme` answer, a
  theme change made offline, and the old local appearance (sent to the server once, if no device ever chose).
- `theme`: the server's theme. The palette comes from the server's
  tokens (the built-in paper theme until it answers); fetched when the state's `theme.at` differs from the cache, applied
  live. A preset or the appearance shows at once and works offline from the cached presets (sent when the server answers
  again); a custom colour is previewed by the server and needs it; a refusal puts the old one back.
- `talk`: your message on its way in each room, until the room's chat has it (the pending bubble, "Not sent", retry).

Screens read them through `state/hooks.ts` (`useSession`, `useChat`, …).

**Auth.** `AuthSession` (`api/auth.ts`) holds this device's access token in memory and its refresh token, install id and
device number in secure storage. Before a request it renews the access token when it is (nearly) out, through
`POST /api/auth/refresh`. The server turns the refresh token over on every use, so only one renewal is ever in flight and
every concurrent request waits on it. A renewal never takes a caller's abort signal either: a renewal cut off after the
server turned the token over would sign the device out. A 401 with `signIn: true` means sign-in is required (or, while a
refresh token is held, that the access token was stale: the next request renews it). Any other 401 means the device isn't
let in at all. Pairing (`POST /api/auth/pair {code, install, name, model}`) answers with the session's tokens. Players that can't send headers get the session in the address (`api.files.url`), images in a header
(`api.files.source`). The session only ever goes to the paired server: every address the API layer builds must be a path
on it (`isServerPath`: "/…", never "//host" or another origin), so a picture or link in a message can't take it elsewhere.

**What leaves the phone, and what comes in.** Attachments go up as they're picked, the file itself as the body (sent
from disk by expo-file-system's native upload, never read whole into memory; 500 MB at most). HTML reports open in a WebView held to their own page: a strict Content-Security-Policy
first in the page, no navigation at all, and a link opens in the browser only when it's tapped. The live browser view
loads the stream's own web client with the session's Basic credentials and keeps nothing.

**The keyboard.** react-native-keyboard-controller follows the keyboard natively, frame by frame (`KeyboardProvider` in
`App.tsx`). The chat's message box is a `KeyboardStickyView` and its list a `KeyboardChatScrollView`: both rise by the
keyboard's height less the tab bar's, the list stays on the newest message, and a drag down the list takes the keyboard
away with the finger. Every other screen with a text box sits in `KeyboardAvoider`, which pads by however much of it the
keyboard covers. On Android the keyboard covers the tab bar, as on iOS: Material's bottom navigation pads itself by the
window's bottom inset, which with `adjustResize` includes the keyboard's, so it would grow by the keyboard's height and
sit on top of it (and the chat's offset, which reads the bar's height, would push the message box under it).
`patches/react-native-bottom-tabs-1.5.0.patch` (pnpm's `patchedDependencies`, at the repository root) takes the
keyboard's inset out of what the bar sees.

**The rooms sheet.** A form sheet: on iOS its own material shows through (`contentStyle` transparent); Android's sheet
has none, so there `contentStyle`'s color is the sheet's surface, rounded at the top, over a dimmed backdrop that
closes it on a tap. The rooms' counts of what waits on you come from the rows the Needs you tab counts (`roomNeeds` in
`features/chat/rooms.ts`), so they add up to its badge. Each agent but the default one (the state's `agents`) has a room
of its own, listed under Agents with the agent's name and who it works for (Off while disabled); its header says it's an
agent and has its pending approvals, opening that agent's list.

**Approvals.** The state's `approvals` are the pending requests; the ones waiting on you count in the Needs you badge
(and in their agent's room), and Needs you's Approvals row opens the list (`GET /api/approvals?state=pending`): what
waits on you, then what waits on an approver, read-only. A request's page fetches it by id and shows its exact payload
(the text word for word in a fixed-width, selectable block, each file's name, size and the first 12 hex digits of its
SHA-256, a payment's amount); Approve (after a confirmation) and Reject send the payload hash of what is on screen, so a
request that changed meanwhile decides nothing (409) and is shown again as it is. One answer goes at a time, a
confirmation that outlives a re-read of the request decides nothing, and an answer lost on its way says it may or may not
have gone through and reads the request again (the logic is `features/approvals/decision.ts`). A waiting request, or an
approved one not used yet, can be withdrawn. Payments, passwords and codes, secrets,
account security and anything in your name are marked as only yours to approve. Characters that would change how a
request's words read without showing themselves (Unicode's format characters, such as bidi overrides and zero-width
characters, and control characters but a line break and a tab) are written out as `[U+202E]`, as on the server's link
page and in Slack (`visible` in `features/approvals/model.ts`).

**Live streams.** `GET /api/live?room=`, `GET /api/runner/sessions/<id>/live?after=` and a party's
`GET /api/parties/<id>/live` are server-sent events, read with expo/fetch's streaming bodies and parsed by
`api/sse.ts`. `followRoomLive`, `followSession` and `followParty` keep a stream open while a screen wants it, reconnect
2 s after a drop and drop a connection that goes 45 s without a byte (the server pings every 15 s). A session's stream
asks only for the steps after the last one seen when it reconnects. When a room's turn ends (its stream sends `null`),
`useRoomLive` fetches the state again so the answer and the end of the working sign arrive together. Its stream's
`approvals` event (a request changed) fetches the states and the approvals pages again; any other named event is left
alone.

**Adding a screen.** Put it in its feature's folder, add its params to `navigation/types.ts` and register it in
`RootStack.tsx` (or `Tabs.tsx`). Build it from `@/ui` and `useTheme()`; take data from the hooks in `state/`.
