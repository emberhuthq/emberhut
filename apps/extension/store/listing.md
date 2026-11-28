# The Chrome Web Store listing

What the extension's store listing and privacy form say, kept with the code they describe so a change to the
extension's permissions or data changes them in the same pull request. The images are made from the real extension
paired with a test hut.

## Store listing

- **Name:** Emberhut (from the manifest)
- **Summary** (the manifest's `description`, 132 characters at most): Lets your own Emberhut assistant work in Chrome,
  only in its own tab group, paired with the Emberhut hut you run yourself.
- **Category:** Productivity › Workflow & Planning
- **Language:** English
- **Homepage:** https://emberhut.com
- **Support:** https://github.com/emberhuthq/emberhut/issues
- **Privacy policy:** https://emberhut.com/privacy#chrome-extension
- **Visibility:** Public. Anyone can install it; it does nothing until it pairs with a hut its owner runs and confirms.
- **Icon:** `icons/emberhut-128.png` (96 px artwork, 16 px transparent padding)
- **Screenshots** (1280×800): the consent page before pairing; the assistant working in its tab group with "Emberhut
  is driving · Pause" on the page; the popup while it works; the popup paused; the popup's settings (blocked sites,
  notifications).
- **Small promo tile** (440×280): the lockup and "Your ember works in Chrome, only in its own tab group."

### Description

A personal AI assistant you host yourself can now work in Chrome for you, as you, but only in its own tab group. This
is the browser half of Emberhut: its server, the hut, runs on your own computer, and this extension pairs with it.

What it does
• Works only in the blue "Emberhut" tab group: tabs your assistant opened, a tab you share with it from the toolbar
popup, and windows those tabs open (a sign-in window, say). Your other tabs are left alone.
• Shows "Emberhut is driving · Pause" on every page it works in, with its own cursor, so you see when it acts.
• Pause from the page, the toolbar or the app: it lets go of every tab at once and comes back by itself after 10
minutes, or as soon as you hand back. Cancel on Chrome's debugging bar pauses it too.
• Hands the browser to you when a step needs you. Your hut's app asks you before any final step (submit, send, pay,
delete).

What it won't do
• Type into password or one-time-code fields: the extension refuses.
• Work on blocked sites: password managers, account security pages, the Chrome Web Store, and any site you add.
• Use Chrome's cookie, storage, network or browser controls: only page-level DevTools commands get through.
• Send anything from your other tabs, or anything at all to Symploc, its maker.

Where your data goes
The pages in the tab group (their addresses, text and screenshots) go to your own hut over an encrypted link (https,
or your own computer or Tailscale address). Your hut keeps them on the computer it runs on and passes what a task needs
to the AI model you chose for it, such as Claude under your own account. Before the first pairing the extension shows
you all of this and waits for your OK.

You need a hut: the desktop app runs one for you, or run the open-source server yourself
(github.com/emberhuthq/emberhut). Without one the extension stays idle. Open source under MIT or Apache-2.0; the
extension's code is in apps/extension, unminified, as it runs.

## Privacy practices

### Single purpose

Lets the user's own self-hosted Emberhut assistant operate web pages in Chrome on their behalf, only in the extension's
"Emberhut" tab group, paired with the user's own Emberhut server (their "hut").

### Permission justifications

- **debugger:** The user's hut works in a web page the way browser automation does, through the Chrome DevTools
  Protocol: it reads the page and clicks, types and runs script in it. chrome.debugger is attached only to tabs in the
  extension's "Emberhut" tab group (or tabs they opened), and only an allow-list of page-level commands is passed on:
  no DevTools domain that reaches the browser's cookie store, storage, network or other targets (no Network, Storage,
  Browser or Target commands). Script the hut runs in a page has only that page's own access. Typing into password and
  one-time-code fields is refused.
- **tabs:** To read the addresses of tabs so the extension keeps to its own tab group, checks every command against the
  blocked sites (password managers, account security pages, the user's own list), and recognises the pairing link the
  user opens from the Emberhut app. Addresses of other tabs are only checked on the device, never sent.
- **tabGroups:** To create and keep the blue "Emberhut" tab group, which marks the only tabs the assistant may work in,
  and to notice when the user drags a tab in or out of it.
- **webNavigation:** To let go of a tab before a navigation to a blocked site commits, in the page or in a frame, so the
  assistant never acts on those sites.
- **storage:** To keep the pairing with the user's hut, the user's blocked sites and a pause across browser restarts,
  and the session's list of recent steps for the popup.
- **alarms:** To reconnect to the user's hut after Chrome suspends the service worker, and to end a 10-minute pause on
  time.
- **notifications (optional):** Requested only when the user turns on Chrome notifications in the popup, to tell them
  when the assistant hands a tab to them or takes it back.
- **Host permissions:** none.

### Remote code

Yes. The extension's own pages and service worker run only the files in its package (CSP `script-src 'self'`, no eval).
The user's own hut sends Chrome DevTools Protocol commands for the pages in the extension's tab group, some of which
run script in that page (`Runtime.evaluate`, `Runtime.callFunctionOn`, `Page.addScriptToEvaluateOnNewDocument`),
through chrome.debugger, the API the policy allows for this. The commands come only from the hut the user paired with
and confirmed; an allow-list in the extension refuses DevTools domains beyond the page, and typing into password fields
is refused.

### Data usage

What the extension handles, sent only to the user's own hut (a server the user runs), never to the developer. The hut
passes what a task needs to the AI model the user chose (for example Anthropic's Claude, under the user's own account).

- [x] **Web history:** the addresses of the pages in the extension's "Emberhut" tab group, while the assistant works in
      them.
- [x] **Website content:** the text, structure and screenshots of the pages in that tab group, and what the assistant
      clicks and types there.
- [x] **Personally identifiable information, Personal communications, Financial and payment information:** only as far
      as a page in the tab group shows them (an inbox, an account page, a checkout), as part of its content.
- [ ] Health information
- [ ] Authentication information (password and one-time-code fields are refused; cookies are never asked for)
- [ ] Location
- [ ] User activity (the extension doesn't record the user's own clicks, keys or scrolling)

Certifications, all three:

- [x] I do not sell or transfer user data to third parties, outside of the approved use cases.
- [x] I do not use or transfer user data for purposes that are unrelated to my item's single purpose.
- [x] I do not use or transfer user data to determine creditworthiness or for lending purposes.

**Privacy policy:** https://emberhut.com/privacy#chrome-extension (with the Limited Use statement).

## Test instructions for the reviewer

The extension works with an Emberhut hut, which the user runs. With a clone of github.com/emberhuthq/emberhut and Rust:

1. `cargo run -p emberhut-server` (listens on http://127.0.0.1:18990).
2. In another terminal, sign a device in and get a Chrome pairing link:
   `code=$(cargo run -q -p emberhut-server -- pair --json | jq -r .code)`, then
   `token=$(curl -s -X POST http://127.0.0.1:18990/api/auth/pair -H 'Content-Type: application/json' -d "{\"code\":\"$code\"}" | jq -r .access)`,
   then
   `curl -s -X POST http://127.0.0.1:18990/api/chrome -H "Authorization: Bearer $token" -H 'Content-Type: application/json' -d '{"kind":"add","label":"Work"}' | jq -r .pairing.link`.
3. Open that link in Chrome with the extension installed. The extension opens its consent page: press Pair. The
   popup now says "Paired as “Work” · connected". (The Emberhut desktop app does steps 1 and 2 for a user.)
4. Settings in the popup: blocked sites, optional notifications, Unpair. Pause in the popup or on a page lets go of
   every tab.

Without a hut, the popup says "Not paired yet" and the extension does nothing else.
