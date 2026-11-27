import type { TextStyle } from "react-native";

export const spacing = { xs: 4, sm: 8, md: 12, lg: 16, xl: 24, xxl: 32 } as const;

export const radius = { sm: 8, md: 12, lg: 16, pill: 999 } as const;

/** The type scale, in the system font, as iOS sizes it (body 17). */
export const type = {
  largeTitle: { fontSize: 30, lineHeight: 36, fontWeight: "700" },
  title: { fontSize: 22, lineHeight: 28, fontWeight: "700" },
  headline: { fontSize: 17, lineHeight: 22, fontWeight: "600" },
  body: { fontSize: 17, lineHeight: 22, fontWeight: "400" },
  callout: { fontSize: 15, lineHeight: 20, fontWeight: "400" },
  footnote: { fontSize: 13, lineHeight: 18, fontWeight: "400" },
  caption: { fontSize: 12, lineHeight: 16, fontWeight: "400" },
} as const satisfies Record<string, TextStyle>;

export type TypeVariant = keyof typeof type;

export type TypeScale = { readonly [K in TypeVariant]: { fontSize: number; lineHeight: number; fontWeight: TextStyle["fontWeight"] } };

/** The scale for a platform: Android's body is Material's 16, everything else as iOS has it. */
export const typeFor = (os: string): TypeScale => (os === "ios" ? type : { ...type, body: { ...type.body, fontSize: 16 } });

/** The smallest a tap target gets. */
export const minTap = 44;
