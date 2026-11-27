import { Image } from "react-native";

const face = require("../../assets/emberhut-face.png");

/** Emberhut's face: the ember, a glowing coal with a charcoal base, with a cream rim for dark themes. */
export function Face({ size = 30 }: { size?: number }) {
  return (
    <Image
      source={face}
      style={{ width: size, height: size, borderRadius: size / 2 }}
      accessibilityIgnoresInvertColors
      importantForAccessibility="no"
    />
  );
}
