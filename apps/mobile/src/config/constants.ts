import { LINK_SCHEME } from "@/api/wire";

/** The app's links (a task, a room, a session), as pushes and messages carry them. */
export const linkSchemes = [LINK_SCHEME] as const;

/** A link the app opens itself: one of its own schemes. */
export const isAppLink = (url: string): boolean => linkSchemes.some((scheme) => url.startsWith(scheme));

/** Main, the room every install starts in. */
export const mainRoom = 1;
