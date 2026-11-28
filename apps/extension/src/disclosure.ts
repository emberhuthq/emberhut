// What the extension tells the owner before it pairs (the consent page, the popup), word for word as the store listing
// and the privacy policy say it: what it sends, to whom, and what it never touches.

/** The disclosure's lines, for a hut at `server`. */
export function disclosure(server: string): string[] {
  return [
    `Pairing connects this Chrome profile to your own Emberhut hut at ${server}, and lets that hut work in Chrome for you.`,
    "Your ember works only in the blue “Emberhut” tab group (and in tabs those tabs open). For that, the extension sends your hut the addresses, text and screenshots of those pages, and your hut clicks, types and runs scripts in them.",
    "Your hut keeps what it gets on the computer it runs on, and passes what a task needs (page text and screenshots included) to the AI model you chose for your ember, such as Claude under your own account, under that provider's terms.",
    "The extension sends nothing from your other tabs and never asks Chrome for your cookies or history. It refuses to type into password and one-time-code fields. Nothing goes to Symploc, the maker of Emberhut.",
    "Your hut also learns this profile's Chrome version and system, to show it in the Emberhut app.",
  ];
}

/** The one line the popup shows before it opens the consent page. */
export const SHORT =
  "Pairing lets your own hut work in the “Emberhut” tab group: it gets those pages' addresses, text and screenshots, and passes what a task needs to your AI model.";
