import { useMemo } from "react";
import Svg, { Path, Rect } from "react-native-svg";
import { qrPath } from "./qrPath";

/** Modules of light margin around the code, as the QR standard asks for. */
const quietZone = 4;

/**
 * A QR code of ASCII text (a pairing link), drawn as one SVG path. Always dark on white whatever the theme: scanners read
 * that best. Nothing is drawn for text it can't encode.
 */
export function QrCode({ value, size, label }: { value: string; size: number; label: string }) {
  const qr = useMemo(() => qrPath(value), [value]);
  if (qr === null) return null;
  const side = qr.size + quietZone * 2;
  return (
    <Svg
      width={size}
      height={size}
      viewBox={`${-quietZone} ${-quietZone} ${side} ${side}`}
      accessibilityRole="image"
      accessibilityLabel={label}
    >
      <Rect x={-quietZone} y={-quietZone} width={side} height={side} fill="#FFFFFF" />
      <Path d={qr.path} fill="#000000" />
    </Svg>
  );
}
