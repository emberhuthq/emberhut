# The Emberhut mark

The mark is the ember: a glowing coal with a charcoal base, dot eyes and a small smile. It is drawn
as a coal lump (soft facets, charcoal flakes, a broken glowing seam), never as a round emoji face,
a flame or a ball.

| File | Use |
| --- | --- |
| `emberhut-mark.svg` | The mark on light grounds, 48 px and up. |
| `emberhut-mark-on-dark.svg` | The same with a cream rim, so the outline holds on dark grounds (it also works on light ones). |
| `emberhut-mark-small.svg` | 16 to 32 px: a thicker outline, bigger eyes, no fine cracks. |
| `emberhut-illustration.svg`, `emberhut-illustration-on-dark.svg` | The full illustration with its glow and sparks, for large use. |
| `emberhut-tile.svg`, `emberhut-tile-dark.svg` | The app tile, light and dark. |
| `emberhut-lockup-on-light.svg`, `emberhut-lockup-on-dark.svg` | The mark beside the wordmark (Baloo 2 ExtraBold, as outlines). |
| `emberhut-stacked-on-light.svg`, `emberhut-stacked-on-dark.svg` | The mark over the wordmark. |
| `social-preview.png` | The repository's social preview, 1280 × 640. |

Colours: outline `#2A2220`; glow `#FFE4A0` → `#FFB24E` → `#FB7E2E` → `#E0501E` → `#C23C16` (centre to
edge); charcoal `#4A403B` → `#231D1B`; seam `#FFC870`; tile `#FFF3E2` → `#FFD9A8` (light),
`#3A2A44` → `#2A2038` (dark).

Where the exports live: the iOS and Android icons in `apps/mobile/assets/`, the macOS icon in
`apps/desktop/assets/AppIcon-1024.png` and `apps/hands/assets/AppIcon.icns`, the Chrome extension's in
`apps/extension/icons/`, and the website's favicon, header mark and link preview in `site/`.

Keep clear space of a quarter of the mark's width around it, don't recolour, stretch or rotate it, and
don't add a flame, arms or a hut to it.

## The ember's tints

The one recolouring allowed is the ember's tint, which a person picks for their own ember (Settings ›
Your ember › Look): Amber, Rose and Sage beside the default, Ember. A tint is `emberhut-mark-on-dark.svg`
with the glow's five stops, the seam, the embers in the charcoal, the cracks and the cheeks swapped for
the tint's colours (`TINTS` in `crates/emberhut-core/src/theme.rs`, whose tests check the face's ink
reads AA on each); the outline, eyes, smile, charcoal and rim stay. The exports are
`apps/desktop/assets/face-<tint>.png` (128 px) and `apps/mobile/assets/emberhut-face-<tint>.png`
(512 px). The app icons stay in the default colours.
