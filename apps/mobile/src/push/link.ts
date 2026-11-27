import { PUSH_CHANNEL } from "@/api";
import { dialect } from "@/api/dialect";
import { isAppLink } from "@/config";

/** The Android channel the server's pushes come on, as the server names it. */
export const channelId = dialect.writeText(PUSH_CHANNEL);

/**
 * What a tap on one of the app's notifications opens: its data's `link`, read as an app link (in the app's words, `words`
 * being the server's dialect); anything else opens nothing.
 */
export function linkOf(data: unknown, words = dialect): string | null {
  if (typeof data !== "object" || data === null) return null;
  const link = (data as { link?: unknown }).link;
  if (typeof link !== "string") return null;
  const own = words.readText(link);
  return isAppLink(own) ? own : null;
}
