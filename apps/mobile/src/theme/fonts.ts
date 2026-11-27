import { Platform } from "react-native";

/** The fixed-width font for code and logs: "monospace" is Android's name for it, iOS has Menlo. */
export const monoFont = Platform.select({ ios: "Menlo", default: "monospace" });
