/** What the first tab calls the assistant; its route stays "Chat", so links to it keep working. */
export const EMBER_TITLE = "Your ember";

/** The ember's two pictures: as it is, and dimmed and grey for off duty. */
export type EmberFaces<Picture> = { readonly on: Picture; readonly off: Picture };

/**
 * Your ember's tab: its title, and the ember itself as its icon on both platforms, drawn in its own colours ("original":
 * the bar doesn't tint it as it does the other tabs' icons). Dimmed while the assistant is off duty; plain until the
 * state says (`onDuty` undefined).
 */
export function emberTabOptions<Picture>(onDuty: boolean | undefined, faces: EmberFaces<Picture>) {
  return {
    title: EMBER_TITLE,
    tabBarIcon: () => (onDuty === false ? faces.off : faces.on),
    tabBarIconRenderingMode: "original" as const,
  };
}
