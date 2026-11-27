import { useState, type ComponentRef, type RefObject } from "react";
import { ActivityIndicator, Platform, StyleSheet, View } from "react-native";
import { WebView } from "react-native-webview";
import type { BrowserSession } from "@/api";
import { Button, Text } from "@/ui";
import { liveViewFailure } from "./browser";
import { injected } from "./scripts";

export type WebViewRef = ComponentRef<typeof WebView>;

/**
 * The stream's own page (its web client) signed in with the session's password, nothing of it kept on the phone. A
 * fresh view per mode: watching and driving get different passwords and scripts. A page that fails to load says why,
 * with Retry, instead of a black screen.
 */
export function LiveView({
  session,
  failed,
  ref,
}: {
  session: BrowserSession | undefined;
  failed: boolean;
  ref: RefObject<WebViewRef | null>;
}) {
  // The failure belongs to the address it happened on: a new one from the server starts clean.
  const [problem, setProblem] = useState<{ url: string; text: string } | null>(null);
  const [attempt, setAttempt] = useState(0);
  if (!session?.url) {
    return (
      <View style={styles.center}>
        {!failed && <ActivityIndicator color="#FFFFFF" />}
        <Text variant="callout" style={styles.note}>
          {failed ? "Emberhut's browser isn't running yet." : "Connecting to Emberhut's browser…"}
        </Text>
      </View>
    );
  }
  const url = session.url;
  if (problem?.url === url) {
    return (
      <View style={styles.center}>
        <Text variant="callout" align="center" style={styles.note} accessibilityLiveRegion="polite">
          {problem.text}
        </Text>
        <Button
          label="Retry"
          variant="secondary"
          onPress={() => {
            setProblem(null);
            setAttempt((n) => n + 1);
          }}
        />
      </View>
    );
  }
  return (
    <WebView
      ref={ref}
      key={`${session.mode}-${attempt}`}
      source={{ uri: session.url }}
      basicAuthCredential={{ username: session.user, password: session.password }}
      injectedJavaScriptBeforeContentLoaded={injected(session.mode, session.screen)}
      incognito
      cacheEnabled={false}
      scrollEnabled={false}
      setBuiltInZoomControls={false}
      overScrollMode="never"
      mediaPlaybackRequiresUserAction={false}
      allowsInlineMediaPlayback
      setSupportMultipleWindows={false}
      onError={(e) => setProblem({ url, text: liveViewFailure(url, e.nativeEvent.description, Platform.OS) })}
      // The page's own answer (react-native-webview reports only the main frame's): a 401 or a 502 says so.
      onHttpError={(e) => setProblem({ url, text: liveViewFailure(url, e.nativeEvent.statusCode, Platform.OS) })}
      style={styles.page}
      accessibilityLabel={session.mode === "control" ? "Emberhut's browser, you're driving" : "Emberhut's browser, live"}
    />
  );
}

const styles = StyleSheet.create({
  center: { flex: 1, alignItems: "center", justifyContent: "center", gap: 8, padding: 24, backgroundColor: "#000000" },
  note: { color: "#A6B0C2" },
  page: { flex: 1, backgroundColor: "#000000" },
});
