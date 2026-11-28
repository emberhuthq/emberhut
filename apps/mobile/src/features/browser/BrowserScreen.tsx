import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as Clipboard from "expo-clipboard";
import { StatusBar } from "expo-status-bar";
import { useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { KeyboardStickyView } from "react-native-keyboard-controller";
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";
import { OWNER, type BrowserInfo, type BrowserInput } from "@/api";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { api } from "@/services";
import { usePrefs } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";
import { queryKeys } from "@/state/query";
import { useTheme } from "@/theme";
import { ActionSheet, Icon, Text, type SheetAction } from "@/ui";
import { BrowserBar } from "./BrowserBar";
import { isMine, isUnclaimed, statusText } from "./browser";
import { useBrowserDevice } from "./browserDevice";
import { DriveKeys } from "./DriveKeys";
import { FileRequestNote } from "./FileRequestNote";
import { LiveView, type WebViewRef } from "./LiveView";
import { keyScript, pasteScript, resetZoomScript } from "./scripts";

const idle: BrowserInfo = { connected: false, url: "", title: "", control: "agent" };

/**
 * Emberhut's browser, live: the whole screen of the Chrome on the server's machine. While an agent drives you watch; Take over gives
 * you the mouse and keyboard and the agents wait; Give back wakes Emberhut. What Emberhut needs you to do shows at its foot.
 */
export function BrowserScreen({ navigation }: RootScreenProps<"Browser">) {
  const { colors, radius, spacing } = useTheme();
  const room = usePrefs((s) => s.room);
  const state = useEmberhutState(room);
  const info = state.data?.browser ?? idle;
  const device = useBrowserDevice();
  const mine = isMine(info, device);
  const web = useRef<WebViewRef>(null);
  const [typing, setTyping] = useState(false);
  const [shortcuts, setShortcuts] = useState(false);
  const client = useQueryClient();
  const insets = useSafeAreaInsets();
  // The foot rides up with the keyboard, over the picture: the screen's own bottom inset is already under it.
  const keyboardOffset = { closed: 0, opened: insets.bottom };

  // Watching and driving get different passwords: a new session when that changes.
  const session = useQuery({
    queryKey: [...queryKeys.browserSession(device ?? ""), mine],
    queryFn: () => api.browser.session(device!),
    enabled: device !== null,
    retry: 1,
  });
  const input = useMutation({
    // A refusal comes as an error status, or as the browser's state with its error in it.
    mutationFn: async (i: BrowserInput) => {
      const after = await api.browser.input(i);
      if (after.error) throw new Error(after.error);
      return after;
    },
    onSettled: () => client.invalidateQueries({ queryKey: queryKeys.allStates }),
  });
  const { mutate } = input;
  const act = (i: BrowserInput) => mutate(i);
  const takeOver = () => device && act({ kind: "takeover", device });

  // Emberhut handed it over and none of your devices has it yet: this one, now it's open here. Only on opening, as in
  // the reference client: a handover that comes while you're watching waits for your Take over.
  const unclaimed = isUnclaimed(info);
  const loaded = state.data !== undefined;
  const checkedOnOpen = useRef(false);
  useEffect(() => {
    if (checkedOnOpen.current || !device || !loaded) return;
    checkedOnOpen.current = true;
    if (unclaimed) mutate({ kind: "takeover", device });
  }, [unclaimed, device, loaded, mutate]);

  const run = (script: string) => web.current?.injectJavaScript(script);
  const shortcutActions: SheetAction[] = [
    { label: "Find (Ctrl F)", icon: "search", onPress: () => run(keyScript("f", "KeyF", 70, { ctrl: true })) },
    { label: "Select all (Ctrl A)", icon: "scan-outline", onPress: () => run(keyScript("a", "KeyA", 65, { ctrl: true })) },
    { label: "Copy (Ctrl C)", icon: "copy-outline", onPress: () => run(keyScript("c", "KeyC", 67, { ctrl: true })) },
    { label: "Undo (Ctrl Z)", icon: "arrow-undo-outline", onPress: () => run(keyScript("z", "KeyZ", 90, { ctrl: true })) },
    { label: "Zoom in", icon: "add-circle-outline", onPress: () => run(keyScript("=", "Equal", 187, { ctrl: true })) },
    { label: "Zoom out", icon: "remove-circle-outline", onPress: () => run(keyScript("-", "Minus", 189, { ctrl: true })) },
    { label: "Whole page (undo pinch)", icon: "contract-outline", onPress: () => run(resetZoomScript) },
    {
      label: info.chromeUI ? "Hide Chrome's controls" : "Show Chrome's controls",
      icon: "browsers-outline",
      onPress: () => act({ kind: "chrome", show: !info.chromeUI }),
    },
  ];

  return (
    <SafeAreaView edges={["top", "bottom"]} style={[styles.fill, { backgroundColor: "#000000" }]}>
      <StatusBar style="light" />
      <View style={[styles.top, { paddingHorizontal: spacing.sm, paddingVertical: spacing.xs }]}>
        <Pressable
          onPress={() => navigation.goBack()}
          accessibilityRole="button"
          accessibilityLabel="Close the browser"
          style={[styles.close, { backgroundColor: colors.surface }]}
        >
          <Icon name="close" color={colors.text} />
        </Pressable>
        <View style={[styles.pill, { gap: spacing.xs + 2, borderRadius: radius.pill, backgroundColor: colors.surface }]}>
          <View
            style={[
              styles.dot,
              { backgroundColor: info.control === OWNER ? colors.needsYou : info.driver ? colors.warning : colors.textTertiary },
            ]}
          />
          <Text variant="footnote" weight="600" numberOfLines={1}>
            {statusText(info, device)}
          </Text>
        </View>
        <View style={styles.close} />
      </View>
      <View style={styles.fill}>
        <LiveView
          ref={web}
          session={session.data}
          failed={session.isError}
          onRetry={() => inBackground(session.refetch(), "BrowserScreen: refetch session")}
        />
      </View>
      {/* Everything at the foot (a refusal, the handoff note, the typing row, the address bar) stays above the keyboard,
          which would cover it otherwise: the field you type in would be out of sight. It rides over the picture instead
          of shrinking it, as a shorter view would have the stream resize the far screen. */}
      <KeyboardStickyView offset={keyboardOffset}>
        {input.error && (
          <Pressable
            onPress={() => input.reset()}
            accessibilityRole="button"
            accessibilityLabel={`${input.error.message}. Tap to hide.`}
            style={[
              styles.row,
              {
                gap: spacing.sm,
                margin: spacing.sm,
                marginBottom: 0,
                padding: spacing.md,
                borderRadius: radius.lg,
                backgroundColor: colors.surface,
              },
            ]}
          >
            <Icon name="alert-circle" color={colors.danger} />
            <Text variant="callout" tone="danger" style={styles.fill}>
              {input.error.message}
            </Text>
          </Pressable>
        )}
        {(mine && info.handoff) || info.fileRequest ? (
          <View style={{ gap: spacing.xs + 2, padding: spacing.sm }}>
            {mine && info.handoff && (
              <View
                style={[styles.row, { gap: spacing.sm, padding: spacing.md, borderRadius: radius.lg, backgroundColor: colors.surface }]}
              >
                <Icon name="hand-left" color={colors.needsYou} />
                <Text variant="callout" weight="500" tone="needsYou" style={styles.fill}>
                  {`Emberhut asked you: ${info.handoff}. Give back when you're done.`}
                </Text>
              </View>
            )}
            {info.fileRequest && (
              <FileRequestNote
                onSent={() => inBackground(client.invalidateQueries({ queryKey: queryKeys.allStates }), "BrowserScreen: invalidateQueries")}
              />
            )}
          </View>
        ) : null}
        {typing && mine && (
          <View style={{ padding: spacing.sm, backgroundColor: colors.surface }}>
            <DriveKeys run={run} />
          </View>
        )}
        <BrowserBar
          info={info}
          mine={mine}
          act={act}
          onTakeOver={takeOver}
          typing={typing}
          onTyping={() => setTyping((t) => !t)}
          onPaste={() =>
            inBackground(
              Clipboard.getStringAsync().then((text) => text && run(pasteScript(text))),
              "BrowserScreen: getStringAsync",
            )
          }
          onShortcuts={() => setShortcuts(true)}
        />
      </KeyboardStickyView>
      <ActionSheet visible={shortcuts} title="Shortcuts" actions={shortcutActions} onClose={() => setShortcuts(false)} />
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  row: { flexDirection: "row", alignItems: "center" },
  top: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  close: { width: 40, height: 40, borderRadius: 20, alignItems: "center", justifyContent: "center" },
  pill: { flexDirection: "row", alignItems: "center", paddingHorizontal: 12, paddingVertical: 6, flexShrink: 1 },
  dot: { width: 7, height: 7, borderRadius: 4 },
});
