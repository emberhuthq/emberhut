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

/**
 * What a tapped notification opens, from its request's content data and trigger. Android puts the server's data in the
 * content, or only in FCM's own message (`trigger.remoteMessage.data`) when the system showed it with the app closed. On
 * an iPhone the content's data is only an Expo push's `body`, so the server's keys, beside `aps`, are in the push
 * trigger's `payload`.
 */
export function tappedLink(data: unknown, trigger: unknown, words = dialect): string | null {
  return linkOf(data, words) ?? linkOf(pushData(trigger), words);
}

function pushData(trigger: unknown): unknown {
  if (typeof trigger !== "object" || trigger === null) return undefined;
  if ("remoteMessage" in trigger) {
    const message = (trigger as { remoteMessage?: { data?: unknown } | null }).remoteMessage;
    return message?.data;
  }
  if ("payload" in trigger) return (trigger as { payload?: unknown }).payload;
  return undefined;
}
