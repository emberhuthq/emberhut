import type { ImageSourcePropType } from "react-native";

// The ember's face in each tint (512 px): the brand's mark with the tint's colours (assets/brand/README.md). Metro bundles
// only what a literal require names, so each tint is listed here; one the server lists that this build has no picture
// of draws as the default.

/** The tint the face is drawn in when none was chosen, or the server is older: the brand's own colours. */
const DEFAULT_TINT = "ember";

/** The face in each tint this build has, by the tint's id. */
export const emberFaceImages: Readonly<Record<string, ImageSourcePropType>> = {
  ember: require("../../assets/emberhut-face.png"),
  amber: require("../../assets/emberhut-face-amber.png"),
  rose: require("../../assets/emberhut-face-rose.png"),
  sage: require("../../assets/emberhut-face-sage.png"),
};

/** The face off duty, the same for every tint: grey at half opacity (128 px). */
export const emberFaceOff: ImageSourcePropType = require("../../assets/emberhut-face-off.png");

/** The face in `tint`, or the default's. */
export function emberFace(tint: string | undefined): ImageSourcePropType {
  return emberFaceImages[tint ?? DEFAULT_TINT] ?? emberFaceImages[DEFAULT_TINT]!;
}
