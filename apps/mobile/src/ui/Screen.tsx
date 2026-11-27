import type { ReactNode } from "react";
import { RefreshControl, ScrollView, StyleSheet, View, type StyleProp, type ViewStyle } from "react-native";
import { SafeAreaView, type Edge } from "react-native-safe-area-context";
import { useTheme } from "@/theme";
import { useBarInsets } from "./barInsets";

export interface ScreenProps {
  children: ReactNode;
  /** A scrolling page; off for screens that hold their own list. */
  scroll?: boolean;
  /** Pull to refresh (scrolling pages only). */
  refreshing?: boolean;
  onRefresh?: () => void;
  /** The safe-area edges to pad: a screen under a header or above the tab bar needs fewer. */
  edges?: Edge[];
  padded?: boolean;
  /**
   * Its own list runs under the see-through bars and handles them itself; otherwise a scrolling page scrolls under them
   * (iOS insets it) and a fixed one starts below them.
   */
  underBars?: boolean;
  /** No background of its own: a sheet's material shows through. */
  transparent?: boolean;
  style?: StyleProp<ViewStyle>;
}

export function Screen({
  children,
  scroll = false,
  refreshing = false,
  onRefresh,
  edges = [],
  padded = true,
  underBars = false,
  transparent = false,
  style,
}: ScreenProps) {
  const theme = useTheme();
  const bars = useBarInsets();
  const padding = padded ? theme.spacing.lg : 0;
  const background = transparent ? undefined : theme.colors.background;
  return (
    <SafeAreaView edges={edges} style={[styles.fill, { backgroundColor: background }]}>
      {scroll ? (
        <ScrollView
          contentContainerStyle={[{ padding, gap: theme.spacing.md }, style]}
          // Under the see-through header and tab bar iOS insets the page itself, so a large title shrinks as it scrolls.
          contentInsetAdjustmentBehavior="automatic"
          refreshControl={
            onRefresh ? <RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={theme.colors.accent} /> : undefined
          }
        >
          {children}
        </ScrollView>
      ) : (
        <View
          style={[
            styles.fill,
            { padding },
            style,
            underBars ? null : { paddingTop: padding + bars.top, paddingBottom: padding + bars.bottom },
          ]}
        >
          {children}
        </View>
      )}
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({ fill: { flex: 1 } });
