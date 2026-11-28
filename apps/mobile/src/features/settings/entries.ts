/**
 * Every setting, described once, as the desktop's list describes it (`apps/desktop/src/store/settings/entries.rs`): its id,
 * title, one line (fixed, or worked out from how things stand), its group and where in it, the other words people call it
 * by, whether it is for every day or technical, and where opening it goes. Settings' search and its suggestions both read
 * this list. The ids and groups are checked against `apps/fixtures/settings-entries.json`, which the desktop's are
 * checked against too, so the two lists can't drift.
 */

import type { IconName } from "@/ui";

/** Settings' groups, in the chips' order. Account shows only in builds with the emberhut.com account. */
export const groups = ["ember", "appearance", "devices", "connections", "sharing", "privacy", "advanced", "account"] as const;

export type GroupId = (typeof groups)[number];

/** Each group's name, as its chip and its screen's title say it. */
export const groupName: Record<GroupId, string> = {
  ember: "Your ember",
  appearance: "Appearance",
  devices: "Devices",
  connections: "Connections",
  sharing: "Sharing",
  privacy: "Privacy",
  advanced: "Advanced",
  account: "Account",
};

/** Each group's icon, on its row. */
export const groupIcon: Record<GroupId, IconName> = {
  ember: "flame-outline",
  appearance: "sunny-outline",
  devices: "phone-portrait-outline",
  connections: "git-network-outline",
  sharing: "people-outline",
  privacy: "shield-outline",
  advanced: "options-outline",
  account: "person-circle-outline",
};

/** How the things Settings speaks of stand now: what the live lines, the suggestions and the status lines read. */
export interface Now {
  onDuty: boolean;
  /** Your own name for the ember; undefined while it is "Your ember". */
  emberName?: string;
  /** Whether it is at work on something now. */
  working: boolean;
  /** How many background jobs it is running. */
  jobs: number;
  /** What the agent thinks with (`DeepSeek · key ending 0000`), once read. */
  provider?: string;
  /** How many facts it keeps about you, once read. */
  facts?: number;
  /** How many lessons it is trying out, waiting on you. */
  lessons: number;
  /** Whether the app shows dark now. */
  dark: boolean;
  /** The appearance chosen. */
  appearance: "system" | "light" | "dark";
  /** How many devices are signed in, when the server says. */
  devices?: number;
  /** Whether a device other than this phone is signed in. */
  otherDevices: boolean;
  signedIn: boolean;
  /** The tool servers that can't connect. */
  failingTools: string[];
  /** The data sources that need a look. */
  sourcesToFix: string[];
  /** How many data sources are working. */
  sourcesOn: number;
  /** Whether this build has the emberhut.com account. */
  hasAccount: boolean;
  /** Whether the rooms list shows the agents' rooms (this device's choice). */
  showAgents: boolean;
  /** Where the agents' rooms sit in the rooms list: after the paused fold, or before it. */
  agentsPlace: "below" | "above";
}

/** Nothing known yet. */
export const nothingKnown: Now = {
  onDuty: false,
  working: false,
  jobs: 0,
  lessons: 0,
  dark: false,
  appearance: "system",
  otherDevices: false,
  signedIn: false,
  failingTools: [],
  sourcesToFix: [],
  sourcesOn: 0,
  hasAccount: false,
  showAgents: true,
  agentsPlace: "below",
};

/** Where opening a setting goes: its group's screen, or a screen of its own. */
export type Destination = { group: GroupId } | { screen: "Credentials" | "AboutYou" | "Machines" | "Account" };

export interface Entry {
  /** Its id: the desktop's list uses the same. */
  id: string;
  title: string;
  line: string | ((now: Now) => string);
  group: GroupId;
  /** Where in its group, after the group's name, if not at its top. */
  place?: string;
  /** Other words people use for it. */
  synonyms: readonly string[];
  technical?: boolean;
  to: Destination;
  icon: IconName;
}

/** Its line now. */
export function lineOf(entry: Entry, now: Now): string {
  return typeof entry.line === "string" ? entry.line : entry.line(now);
}

/** Where it lives, as a result says it: `Connections › Tools`. */
export function pathOf(entry: Entry): string {
  return entry.place ? `${groupName[entry.group]} › ${entry.place}` : groupName[entry.group];
}

/** The appearance chosen, in a word: `Light`, `Dark`, `As your phone is`. */
export function appearanceWord(now: Now): string {
  return now.appearance === "light" ? "Light" : now.appearance === "dark" ? "Dark" : "As your phone is";
}

const counted = (count: number, one: string) => `${count} ${one}${count === 1 ? "" : "s"}`;

/** Every setting, in the order a tie in search keeps (the desktop's order). */
export const entries: readonly Entry[] = [
  {
    id: "on-duty",
    title: "On duty",
    line: (now) => (now.onDuty ? "On duty · works on its own" : "Off duty · still answers when you message it"),
    group: "ember",
    synonyms: ["pause", "resume", "off duty", "stop", "sleep", "wake", "status", "connection"],
    to: { group: "ember" },
    icon: "power-outline",
  },
  {
    id: "ember-name",
    title: "Name",
    line: (now) => (now.emberName ? `Called ${now.emberName} · rename it` : "Give your ember a name of your own"),
    group: "ember",
    synonyms: ["name", "rename", "call it", "nickname", "what to call", "assistant name", "change name"],
    to: { group: "ember" },
    icon: "pencil-outline",
  },
  {
    id: "thinks-with",
    title: "What your ember thinks with",
    line: (now) => now.provider ?? "Claude, OpenAI, DeepSeek, OpenRouter, Ollama…",
    group: "ember",
    place: "What it thinks with",
    synonyms: ["model", "provider", "api key", "key", "llm", "claude", "openai", "deepseek", "openrouter", "ollama"],
    to: { screen: "Credentials" },
    icon: "key-outline",
  },
  {
    id: "usage",
    title: "Claude usage",
    line: "How much of each account's week is used, and which one your ember runs on",
    group: "ember",
    place: "What it thinks with",
    synonyms: ["limits", "quota", "accounts", "auto", "pin", "week", "5-hour", "check now", "refresh"],
    to: { group: "ember" },
    icon: "bar-chart-outline",
  },
  {
    id: "about-you",
    title: "What your ember knows about you",
    line: (now) => (now.facts === undefined ? "Fix or forget any of it" : `${counted(now.facts, "thing")} · fix or forget any`),
    group: "ember",
    place: "What it knows",
    synonyms: ["memory", "forget", "facts", "remember", "about me", "fix a fact", "edit"],
    to: { screen: "AboutYou" },
    icon: "book-outline",
  },
  {
    id: "lessons",
    title: "Learning from you",
    line: (now) => (now.lessons === 0 ? "Lessons from your corrections: keep or drop each" : `${counted(now.lessons, "lesson")} to review`),
    group: "ember",
    place: "What it knows",
    synonyms: ["trial", "corrections", "keep", "drop", "trying out"],
    to: { screen: "AboutYou" },
    icon: "school-outline",
  },
  {
    id: "models",
    title: "Technical details",
    line: "The model and the side rooms' fast model",
    group: "ember",
    synonyms: ["fast model", "side room", "session"],
    technical: true,
    to: { group: "ember" },
    icon: "hardware-chip-outline",
  },
  {
    id: "appearance-mode",
    title: "Dark appearance",
    line: "Light · Dark · As your phone is",
    group: "appearance",
    synonyms: ["theme", "dark mode", "light", "light mode", "night", "system", "mode"],
    to: { group: "appearance" },
    icon: "moon-outline",
  },
  {
    id: "colour",
    title: "Colour",
    line: "Ember on paper and others, in light and dark",
    group: "appearance",
    synonyms: ["color", "accent", "preset", "paper", "theme"],
    to: { group: "appearance" },
    icon: "color-palette-outline",
  },
  {
    id: "custom-colour",
    title: "Custom colour",
    line: "Your own accent colour, with a preview",
    group: "appearance",
    place: "Colour",
    synonyms: ["hex", "color", "accent", "preview"],
    technical: true,
    to: { group: "appearance" },
    icon: "eyedrop-outline",
  },
  {
    id: "show-agents",
    title: "Show agents",
    line: (now) => (now.showAgents ? "Your agents' rooms, in the rooms list" : "Hidden · open them from here"),
    group: "appearance",
    place: "Rooms list",
    synonyms: ["hide agents", "hide", "agents", "agent rooms", "sidebar", "rooms list"],
    to: { group: "appearance" },
    icon: "people-circle-outline",
  },
  {
    id: "agents-place",
    title: "Where agents sit",
    line: (now) => (now.agentsPlace === "below" ? "Below rooms, after paused ones" : "Before paused rooms"),
    group: "appearance",
    place: "Rooms list",
    synonyms: ["agents below rooms", "agents above rooms", "below", "above", "move agents", "order", "sidebar order", "agents"],
    to: { group: "appearance" },
    icon: "swap-vertical-outline",
  },
  {
    id: "devices",
    title: "Devices",
    line: (now) =>
      now.devices === undefined ? "Each device, when it was last seen, Sign out" : `${now.devices} can reach your ember · sign one out`,
    group: "devices",
    synonyms: ["phone", "computer", "sign out", "signed in", "iphone", "android", "mac"],
    to: { group: "devices" },
    icon: "phone-portrait-outline",
  },
  {
    id: "pair",
    title: "Add a phone or computer",
    line: "One code (and QR), works once for 10 minutes",
    group: "devices",
    synonyms: ["pair", "qr", "code", "link", "new device", "pair another"],
    to: { group: "devices" },
    icon: "qr-code-outline",
  },
  {
    id: "sign-in-here",
    title: "Sign this phone in",
    line: "It pairs with a code from a signed-in device, from the sign-in screen",
    group: "devices",
    synonyms: ["pairing code", "pairing link", "pair this computer", "log in"],
    to: { group: "devices" },
    icon: "log-in-outline",
  },
  {
    id: "machines",
    title: "Machines",
    line: "The computers your ember works on",
    group: "devices",
    place: "Computers your ember works on",
    synonyms: ["computers", "runner", "helpers", "cpu", "gpu", "memory"],
    to: { screen: "Machines" },
    icon: "hardware-chip-outline",
  },
  {
    id: "sources",
    title: "Data sources",
    line: "Gmail, Slack, Chrome… each does only what it says",
    group: "connections",
    synonyms: ["gmail", "slack", "mail", "email", "connect", "unlink", "reconnect", "connections"],
    to: { group: "connections" },
    icon: "git-network-outline",
  },
  {
    id: "chrome",
    title: "Chrome profiles",
    line: "Your ember reads and clicks only in its own tab group",
    group: "connections",
    place: "Chrome",
    synonyms: ["chrome", "browser", "profile", "tab group", "pair a profile"],
    to: { group: "connections" },
    icon: "globe-outline",
  },
  {
    id: "tools",
    title: "Tool servers (MCP)",
    line: "Switch each on or off; reconnect one that can't connect",
    group: "connections",
    place: "Tools",
    synonyms: ["mcp", "tools", "servers", "reconnect", "plugins", "integrations"],
    technical: true,
    to: { group: "connections" },
    icon: "construct-outline",
  },
  {
    id: "skills",
    title: "Skills",
    line: "What your ember knows how to do, each with a switch",
    group: "connections",
    place: "Skills",
    synonyms: ["abilities", "know how"],
    technical: true,
    to: { group: "connections" },
    icon: "sparkles-outline",
  },
  {
    id: "sharing",
    title: "Sharing",
    line: "Only you can talk to your ember · early preview",
    group: "sharing",
    synonyms: ["share", "invite", "others", "people", "team", "access"],
    to: { group: "sharing" },
    icon: "people-outline",
  },
  {
    id: "archived-rooms",
    title: "Archived rooms",
    line: "Open or bring back a room you archived",
    group: "privacy",
    synonyms: ["archive", "unarchive", "old rooms", "restore", "rooms"],
    to: { group: "privacy" },
    icon: "archive-outline",
  },
  {
    id: "dark-room",
    title: "Dark room history",
    line: "Clear everything said off the record",
    group: "privacy",
    synonyms: ["off the record", "clear history", "delete", "incognito"],
    to: { group: "privacy" },
    icon: "lock-closed-outline",
  },
  {
    id: "data-flow",
    title: "Where your data goes",
    line: "What goes to the model you picked, and what stays with you",
    group: "privacy",
    synonyms: ["data", "privacy", "push", "apple", "sent"],
    to: { group: "privacy" },
    icon: "shield-outline",
  },
  {
    id: "updates",
    title: "Updates",
    line: "This app's version",
    group: "advanced",
    synonyms: ["version", "update", "upgrade", "restart", "check for updates", "about"],
    to: { group: "advanced" },
    icon: "refresh-outline",
  },
  {
    id: "server",
    title: "Your ember's home",
    line: "The server's address",
    group: "advanced",
    synonyms: ["server", "address", "url", "host", "port"],
    technical: true,
    to: { group: "advanced" },
    icon: "server-outline",
  },
  {
    id: "phone-push",
    title: "Phone alerts",
    line: "Whether your ember can reach this phone with the app closed",
    group: "devices",
    synonyms: ["notifications", "push", "alerts", "apns"],
    to: { group: "devices" },
    icon: "notifications-outline",
  },
  {
    id: "diagnostics",
    title: "Diagnostics",
    line: "Sign-in mode, this phone's machine id, the connection",
    group: "advanced",
    place: "Diagnostics",
    synonyms: ["sign-in mode", "metrics", "poll", "debug", "machine id", "logs"],
    technical: true,
    to: { group: "advanced" },
    icon: "pulse-outline",
  },
  {
    id: "workplaces",
    title: "Where your ember works",
    line: "Runners, workers, its browser, coding apps",
    group: "advanced",
    synonyms: ["runner", "worker", "browser", "coding app", "environment", "repair", "start"],
    technical: true,
    to: { group: "advanced" },
    icon: "hammer-outline",
  },
  {
    id: "account",
    title: "Account",
    line: "Your emberhut.com account · optional",
    group: "account",
    synonyms: ["sign in", "emberhut.com", "relay", "login", "log in"],
    to: { screen: "Account" },
    icon: "person-circle-outline",
  },
];

/**
 * What the agent is doing, as Your ember's screen says it under its title: `Ready`, `Working on 2 things`, `Off duty`. The On
 * duty switch says whether it is on duty; this doesn't repeat it.
 */
export function doingWord(now: Now): string {
  if (!now.onDuty) return "Off duty";
  const count = Math.max(now.jobs, now.working ? 1 : 0);
  return count === 0 ? "Ready" : `Working on ${counted(count, "thing")}`;
}

/** Whether a group shows now: Account only in builds with the account. */
export function groupShown(group: GroupId, now: Now): boolean {
  return group !== "account" || now.hasAccount;
}

/** The line under a group's title: how it stands, in a few words. */
export function groupStatus(group: GroupId, now: Now): string {
  switch (group) {
    case "ember":
      return doingWord(now);
    case "appearance":
      return appearanceWord(now);
    case "devices":
      return now.devices === undefined ? "" : `${now.devices} can reach your ember`;
    case "connections": {
      const look = now.failingTools.length + now.sourcesToFix.length;
      return [now.sourcesOn > 0 ? `${now.sourcesOn} on` : null, look === 0 ? null : look === 1 ? "1 needs a look" : `${look} need a look`]
        .filter(Boolean)
        .join(" · ");
    }
    case "sharing":
      return "Only you";
    case "privacy":
      return now.facts === undefined ? "" : `${counted(now.facts, "thing")} known`;
    case "advanced":
      return "Nothing here is needed day to day";
    case "account":
      return "Optional";
  }
}
