import { useEffect, useState, type ComponentRef, type RefObject } from "react";
import { ActivityIndicator, Platform, StyleSheet, View } from "react-native";
import { WebView } from "react-native-webview";
import type { BrowserSession } from "@/api";
import { Button, Text } from "@/ui";
import { liveViewFailure, noteStream, streamStalled, watchStream } from "./browser";
import { injected } from "./scripts";

export type WebViewRef = ComponentRef<typeof WebView>;

/** Automatic retries of a stream that can't be reached, before it waits for your Retry. */
const autoRetries = 3;

/**
 * The stream's own page (its web client) signed in with the session's password, nothing of it kept on the phone. A
 * fresh view per mode: watching and driving get different passwords and scripts. A page that fails to load says why,
 * with Retry, instead of a black screen.
 */
export function LiveView({
  session,
  failed,
  onRetry,
  ref,
}: {
  session: BrowserSession | undefined;
  failed: boolean;
  /** Retry asks the server for the session again too: its address or sign-in may have changed. */
  onRetry: () => void;
  ref: RefObject<WebViewRef | null>;
}) {
  // The failure belongs to the address it happened on: a new one from the server starts clean.
  const [problem, setProblem] = useState<{ url: string; text: string } | null>(null);
  const [attempt, setAttempt] = useState(0);
  // The stream behind a page that loaded may still never draw (scripts.ts reports it): a black page says so, and is
  // opened again a few times on its own (each time with its 12 s), then waits for Retry.
  const [watch, setWatch] = useState(() => watchStream(Date.now()));
  const [retried, setRetried] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const url = session?.url;
  // A new address or mode is a new page: its own 12 s, its own retries.
  const page = `${url}|${session?.mode}`;
  const [watched, setWatched] = useState(page);
  if (watched !== page) {
    setWatched(page);
    setWatch(watchStream(now));
    setRetried(0);
  }
  useEffect(() => {
    const clock = setInterval(() => setNow(Date.now()), 2000);
    return () => clearInterval(clock);
  }, []);
  const stalled = url !== undefined && problem?.url !== url && streamStalled(watch, now);
  if (stalled && retried < autoRetries) {
    setRetried(retried + 1);
    setAttempt(attempt + 1);
    setWatch(watchStream(now));
  }
  const retry = () => {
    onRetry();
    setProblem(null);
    setRetried(0);
    setAttempt((n) => n + 1);
    setWatch(watchStream(now));
  };
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
  // Given up (the page is closed then: a refused one would keep asking), or the page's own failure: why, with Retry.
  const gaveUp = stalled && retried >= autoRetries;
  if (problem?.url === session.url || gaveUp) {
    return (
      <View style={styles.center}>
        <Text variant="callout" align="center" style={styles.note} accessibilityLiveRegion="polite">
          {problem?.url === session.url ? problem.text : "Can't reach the browser."}
        </Text>
        <Button label="Retry" variant="secondary" onPress={retry} />
      </View>
    );
  }
  return (
    <View style={styles.page}>
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
        onMessage={(e) => {
          const report = readReport(e.nativeEvent.data);
          if (report) setWatch((w) => noteStream(w, report, Date.now()));
        }}
        onError={(e) => setProblem({ url: session.url!, text: liveViewFailure(session.url!, e.nativeEvent.description, Platform.OS) })}
        // The page's own answer (react-native-webview reports only the main frame's): a 401 or a 502 says so.
        onHttpError={(e) => setProblem({ url: session.url!, text: liveViewFailure(session.url!, e.nativeEvent.statusCode, Platform.OS) })}
        style={styles.page}
        accessibilityLabel={session.mode === "control" ? "Emberhut's browser, you're driving" : "Emberhut's browser, live"}
      />
      {retried > 0 && !watch.pictures && (
        <View style={[StyleSheet.absoluteFill, styles.center]}>
          <ActivityIndicator color="#FFFFFF" />
          <Text variant="callout" align="center" style={styles.note} accessibilityLiveRegion="polite">
            {"Can't reach the browser, retrying…"}
          </Text>
        </View>
      )}
    </View>
  );
}

/** A stream report from the page (scripts.ts), or null for anything else it posts. */
function readReport(data: string): { chunks: number; open: boolean } | null {
  try {
    const m = JSON.parse(data) as { type?: unknown; chunks?: unknown; open?: unknown };
    return m.type === "emberhut-stream" && typeof m.chunks === "number" && typeof m.open === "boolean"
      ? { chunks: m.chunks, open: m.open }
      : null;
  } catch {
    return null;
  }
}

const styles = StyleSheet.create({
  center: { flex: 1, alignItems: "center", justifyContent: "center", gap: 8, padding: 24, backgroundColor: "#000000" },
  note: { color: "#A6B0C2" },
  page: { flex: 1, backgroundColor: "#000000" },
});
