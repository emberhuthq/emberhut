// The wire words: values the API contract spells, each written once here and read from here. A build for a server that
// spells some of them differently rewrites them at the edge (src/api/dialect.ts), so the app only ever sees these. No
// imports, so app.config.ts reads it too.

/**
 * How the server names the person Emberhut works for: a task's owner, a history line's or a session step's author, who
 * controls the browser.
 */
export const OWNER = "owner";

/** How the server names the assistant itself in the same places, and as a task reply's recipient. */
export const ASSISTANT = "assistant";

/** The request header that tells the server which room is on screen (GET /api/state). */
export const LOOKING_HEADER = "X-Emberhut-Looking";

/** The Android notification channel the server's pushes name. */
export const PUSH_CHANNEL = "emberhut";

/** The app's link scheme: the links pushes and messages carry (a task, a room, a session). */
export const LINK_SCHEME = "emberhut://";

/** How the API contract's task keys start: task 12's key is this followed by 12. */
export const CONTRACT_TASK_KEY_PREFIX = "T-";

/**
 * How a task key's prefix may be spelt: letters, then digits or letters, then a dash ("T-", "OT-"). So it holds no regex
 * metacharacters, and the dialect's rule for a prefix ending in - (a digit must follow) applies to it.
 */
const KEY_PREFIX = /^[A-Za-z][A-Za-z0-9]*-$/;

/**
 * The task-key prefix a build shows, from EXPO_PUBLIC_EMBERHUT_TASK_KEY_PREFIX: unset or blank is the contract's. Keys
 * are read into it from every server and written back in the server's own spelling (src/api/dialect.ts). Anything that
 * isn't a prefix throws, which stops the build when app.config.ts reads it.
 */
export function taskKeyPrefixFrom(value: string | undefined): string {
  const given = value?.trim() ?? "";
  if (given === "") return CONTRACT_TASK_KEY_PREFIX;
  if (!KEY_PREFIX.test(given)) throw new Error(`The task-key prefix "${given}" isn't letters and digits ending in a dash, like "T-"`);
  return given;
}

/**
 * How the app writes task keys: task 12's key is this followed by 12. Holds no regex metacharacters. Read as
 * `process.env.EXPO_PUBLIC_…` literally, which Metro inlines when it bundles.
 */
export const TASK_KEY_PREFIX = taskKeyPrefixFrom(process.env.EXPO_PUBLIC_EMBERHUT_TASK_KEY_PREFIX);

/** A task's key as the app writes it, from its number. */
export const taskKey = (id: number | string): string => `${TASK_KEY_PREFIX}${id}`;

/** A task key as a regex source, its number captured: for the patterns that find one in a line. */
export const TASK_KEY_PATTERN = `${TASK_KEY_PREFIX}(\\d+)`;

/** The environments page's group (GET /api/environments) that holds the external threads' apps. */
export const EXTERNAL_THREADS_GROUP = "t3";

/** Where an external thread's latest activity is read (GET, read-only). */
export const EXTERNAL_THREAD_ACTIVITY_PATH = "/api/t3/activity";

/**
 * Where a step of a task's plan is (a plan step's and the current step's `state`): not started, being worked on, done
 * (ticked), stuck on what its note says, left out. An open set: a state the app doesn't know reads as not started.
 */
export const STEP_STATE = { todo: "todo", doing: "doing", done: "done", blocked: "blocked", skipped: "skipped" } as const;

/** One of the plan step states the app knows. */
export type StepState = (typeof STEP_STATE)[keyof typeof STEP_STATE];

/** Who a task, a line or a step is from. */
export type Author = typeof OWNER | typeof ASSISTANT;
