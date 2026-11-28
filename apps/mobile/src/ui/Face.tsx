import { Image } from "react-native";
import { emberFace } from "./emberFaces";

/**
 * Emberhut's face: the ember, a glowing coal with a charcoal base, with a cream rim for dark themes, drawn in `tint`
 * (the theme's tint for the ember; the brand's own colours without one).
 */
export function Face({ size = 30, tint }: { size?: number; tint?: string }) {
  return (
    <Image
      source={emberFace(tint)}
      style={{ width: size, height: size, borderRadius: size / 2 }}
      accessibilityIgnoresInvertColors
      importantForAccessibility="no"
    />
  );
}
