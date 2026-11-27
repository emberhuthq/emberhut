import createQr from "qrcode-generator";

/** A QR code as one SVG path: `size` modules a side, the quiet zone left to the drawing. */
export interface QrPath {
  size: number;
  /** Each run of dark modules in a row as one rectangle, in module units. */
  path: string;
}

/**
 * Encodes ASCII text (a pairing link is, its fragment percent-encoded) as a QR code at error correction level M, the
 * smallest version that holds it. Null for text with anything past ASCII, which the encoder would cut to its low byte.
 */
export function qrPath(text: string): QrPath | null {
  if (text === "" || !/^[\x20-\x7e]*$/.test(text)) return null;
  const code = createQr(0, "M");
  code.addData(text, "Byte");
  code.make();
  const size = code.getModuleCount();
  let path = "";
  for (let row = 0; row < size; row += 1) {
    let col = 0;
    while (col < size) {
      if (!code.isDark(row, col)) {
        col += 1;
        continue;
      }
      const start = col;
      while (col < size && code.isDark(row, col)) col += 1;
      path += `M${start} ${row}h${col - start}v1h${start - col}z`;
    }
  }
  return { size, path };
}
