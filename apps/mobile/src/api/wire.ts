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

/** How the server's task keys start: task 12's key is this followed by 12. Holds no regex metacharacters. */
export const TASK_KEY_PREFIX = "T-";

/** A task's key as the server writes it, from its number. */
export const taskKey = (id: number | string): string => `${TASK_KEY_PREFIX}${id}`;

/** A task key as a regex source, its number captured: for the patterns that find one in a line. */
export const TASK_KEY_PATTERN = `${TASK_KEY_PREFIX}(\\d+)`;

/** The environments page's group (GET /api/environments) that holds the external threads' apps. */
export const EXTERNAL_THREADS_GROUP = "t3";

/** Where an external thread's latest activity is read (GET, read-only). */
export const EXTERNAL_THREAD_ACTIVITY_PATH = "/api/t3/activity";

/** Who a task, a line or a step is from. */
export type Author = typeof OWNER | typeof ASSISTANT;
