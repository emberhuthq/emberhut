import { BlurView } from "expo-blur";
import { GlassView, isLiquidGlassAvailable } from "expo-glass-effect";
import type { ReactNode } from "react";
import { AccessibilityInfo, Platform, StyleSheet, View, type StyleProp, type ViewStyle } from "react-native";
import { useStore } from "zustand";
import { useTheme } from "@/theme";
import { createReduceTransparencyStore } from "./reduceTransparency";

// The system's own materials, used sparingly: the message box. The bars and sheets are the system's own, with theirs.
// Liquid Glass (iOS 26 on) through expo-glass-effect; older iOS gets UIKit's blur materials through expo-blur; Android,
// and anyone with Reduce Transparency on, gets solid surfaces.

const liquid = isLiquidGlassAvailable();

/** The app's one reading of Reduce Transparency, from its first import (App waits for it before drawing screens). */
export const reduceTransparencyStore = createReduceTransparencyStore({
  read: () => AccessibilityInfo.isReduceTransparencyEnabled(),
  listen: (onChange) => {
    const subscription = AccessibilityInfo.addEventListener("reduceTransparencyChanged", onChange);
    return () => subscription.remove();
  },
});

/** Whether the system was asked for less transparency (Settings, Accessibility, Display): then nothing is see-through. */
export const useReduceTransparency = (): boolean => useStore(reduceTransparencyStore, (s) => s.reduce);

export interface GlassProps {
  children?: ReactNode;
  style?: StyleProp<ViewStyle>;
}

/** A floating control's glass, as iOS 26's own floating controls have it: the message box. */
export function Glass({ children, style }: GlassProps) {
  const { colors, scheme } = useTheme();
  const reduce = useReduceTransparency();
  if (Platform.OS === "ios" && !reduce) {
    if (liquid) {
      return (
        <GlassView glassEffectStyle="regular" colorScheme={scheme} style={style}>
          {children}
        </GlassView>
      );
    }
    return (
      <BlurView tint="systemThinMaterial" intensity={100} style={[styles.clip, style]}>
        {children}
      </BlurView>
    );
  }
  return <View style={[{ backgroundColor: colors.surfaceRaised, borderColor: colors.border }, styles.edge, style]}>{children}</View>;
}

const styles = StyleSheet.create({
  clip: { overflow: "hidden" },
  edge: { borderWidth: StyleSheet.hairlineWidth },
});
