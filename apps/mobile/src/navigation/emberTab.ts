import { DEFAULT_EMBER_NAME, shortEmberName } from "@/features/ember/name";

/** What the first tab calls the assistant until you name it; its route stays "Chat", so links to it keep working. */
export const EMBER_TITLE = DEFAULT_EMBER_NAME;

/** The longest a name is under the tab's icon before it is cut with an ellipsis (five tabs share the bar). */
export const TAB_NAME = 12;

/** The ember's two pictures: as it is, and dimmed and grey for off duty. */
export type EmberFaces<Picture> = { readonly on: Picture; readonly off: Picture };

/**
 * Your ember's tab: its title (the name you gave it, cut when long, else "Your ember"), and the ember itself as its icon on both platforms, drawn in its own colours ("original":
 * the bar doesn't tint it as it does the other tabs' icons). Dimmed while the assistant is off duty; plain until the
 * state says (`onDuty` undefined).
 */
export function emberTabOptions<Picture>(onDuty: boolean | undefined, faces: EmberFaces<Picture>, name: string = EMBER_TITLE) {
  return {
    title: shortEmberName(name, TAB_NAME),
    tabBarIcon: () => (onDuty === false ? faces.off : faces.on),
    tabBarIconRenderingMode: "original" as const,
  };
}
