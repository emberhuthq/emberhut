import { useState } from "react";
import { ActivityIndicator, Image, StyleSheet, View, type ImageStyle, type StyleProp } from "react-native";
import { isServerPath } from "@/api";
import { api } from "@/services";
import { useTheme } from "@/theme";
import { Icon } from "@/ui";

export interface ServerImageProps {
  /** A server path (/uploads/…, /api/artifacts/<id>) or an https address. */
  path: string;
  style?: StyleProp<ImageStyle>;
  resizeMode?: "cover" | "contain";
  accessibilityLabel?: string;
}

/** A picture from Emberhut's server, loaded with this device's session; a spinner until it's in, a mark if it can't be. */
export function ServerImage({ path, style, resizeMode = "cover", accessibilityLabel }: ServerImageProps) {
  const { colors } = useTheme();
  // An https picture loads as itself, without the session; only Emberhut's own paths get it; anything else isn't loaded.
  const source = /^https:\/\//.test(path) ? { uri: path } : isServerPath(path) ? api.files.source(path) : null;
  const [state, setState] = useState<"loading" | "shown" | "failed">(source ? "loading" : "failed");
  return (
    <View style={[styles.frame, { backgroundColor: colors.surfaceRaised }, style]}>
      {source && (
        <Image
          source={source}
          style={StyleSheet.absoluteFill}
          resizeMode={resizeMode}
          onLoad={() => setState("shown")}
          onError={() => setState("failed")}
          accessibilityLabel={accessibilityLabel}
        />
      )}
      {state !== "shown" && (
        <View style={[StyleSheet.absoluteFill, styles.center]} pointerEvents="none">
          {state === "loading" ? (
            <ActivityIndicator color={colors.textTertiary} />
          ) : (
            <Icon name="image-outline" color={colors.textTertiary} />
          )}
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  frame: { overflow: "hidden" },
  center: { alignItems: "center", justifyContent: "center" },
});
