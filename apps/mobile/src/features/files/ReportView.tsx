import * as Crypto from "expo-crypto";
import { useState } from "react";
import { Linking } from "react-native";
import { WebView, type WebViewMessageEvent, type WebViewNavigation } from "react-native-webview";
import { useTheme } from "@/theme";
import { isReportPage, sandboxed, tappedLink } from "./files";

/**
 * An HTML report, sandboxed: its own scripts run, but it reaches nothing over the network but pictures (the policy ahead of
 * everything in it), keeps nothing, and never navigates. A tapped link is posted to the app and opens in the browser.
 */
export function ReportView({ html }: { html: string }) {
  const { colors } = useTheme();
  // Fresh for each viewer: a message without it came from the page's own scripts, not from a tap.
  const [token] = useState(() => Crypto.randomUUID());
  const open = (e: WebViewMessageEvent) => {
    const url = tappedLink(e.nativeEvent.data, token);
    if (url) void Linking.openURL(url).catch(() => undefined);
  };
  return (
    <WebView
      source={{ html: sandboxed(html, token), baseUrl: "about:blank" }}
      originWhitelist={["*"]}
      onShouldStartLoadWithRequest={(request: WebViewNavigation) => isReportPage(request.url)}
      onMessage={open}
      setSupportMultipleWindows={false}
      incognito
      cacheEnabled={false}
      allowFileAccess={false}
      javaScriptCanOpenWindowsAutomatically={false}
      style={{ backgroundColor: colors.background }}
    />
  );
}
