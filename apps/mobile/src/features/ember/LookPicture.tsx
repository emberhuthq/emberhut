import { useId } from "react";
import Svg, { Circle, ClipPath, Defs, G, Polygon, Rect } from "react-native-svg";
import type { PageColours, Shown } from "./look";

// The look's small pictures: a preset's page (its paper, a sheet with two lines of ink and an accent button, and an accent
// dot) in light, in dark, or in both split on a diagonal for System, as macOS draws "Auto"; and onboarding's swatch, a
// circle of the preset's paper split with its accent.

/** A preset's page's height. */
export const PAGE_HEIGHT = 40;

/** An id usable in an SVG url(#…): React's, without its punctuation. */
const useSvgId = (): string => `look${useId().replace(/[^A-Za-z0-9]/g, "")}`;

function Page({ c, width }: { c: PageColours; width: number }) {
  const sheet = Math.max(width * 0.6, 30);
  return (
    <>
      <Rect x={0} y={0} width={width} height={PAGE_HEIGHT} rx={6} fill={c.paper} />
      <Rect x={7} y={7} width={sheet} height={PAGE_HEIGHT - 14} rx={4} fill={c.sheet} />
      <Rect x={13} y={13} width={sheet * 0.62} height={3} rx={1.5} fill={c.ink} />
      <Rect x={13} y={19} width={sheet * 0.4} height={3} rx={1.5} fill={c.muted} />
      <Rect x={13} y={25} width={16} height={5} rx={2.5} fill={c.accent} />
      <Circle cx={width - 15} cy={PAGE_HEIGHT / 2} r={5} fill={c.accent} />
    </>
  );
}

/** A preset's page, `width` wide, in the appearance `shown`. */
export function PagePicture({ light, dark, shown, width }: { light: PageColours; dark: PageColours; shown: Shown; width: number }) {
  const clip = useSvgId();
  // System's split: steeper than corner to corner, through the middle; the dark page below it, over the whole light one.
  const lean = Math.min(PAGE_HEIGHT * 0.6, width * 0.25);
  const below = `${width / 2 + lean},0 ${width},0 ${width},${PAGE_HEIGHT} ${width / 2 - lean},${PAGE_HEIGHT}`;
  return (
    <Svg width={width} height={PAGE_HEIGHT} viewBox={`0 0 ${width} ${PAGE_HEIGHT}`}>
      {shown === "both" ? (
        <>
          <Defs>
            <ClipPath id={clip}>
              <Polygon points={below} />
            </ClipPath>
          </Defs>
          <Page c={light} width={width} />
          <G clipPath={`url(#${clip})`}>
            <Page c={dark} width={width} />
          </G>
        </>
      ) : (
        <Page c={shown === "dark" ? dark : light} width={width} />
      )}
    </Svg>
  );
}

/** A preset's swatch: a circle `size` across of its paper, the lower right half its accent, edged in `edge`. */
export function Swatch({ paper, accent, edge, size }: { paper: string; accent: string; edge: string; size: number }) {
  const clip = useSvgId();
  const r = size / 2;
  return (
    <Svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
      <Defs>
        <ClipPath id={clip}>
          <Polygon points={`${size},0 ${size},${size} 0,${size}`} />
        </ClipPath>
      </Defs>
      <Circle cx={r} cy={r} r={r} fill={edge} />
      <Circle cx={r} cy={r} r={r - 1} fill={paper} />
      <Circle cx={r} cy={r} r={r - 1} fill={accent} clipPath={`url(#${clip})`} />
    </Svg>
  );
}
