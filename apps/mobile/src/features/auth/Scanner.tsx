import { CameraView, useCameraPermissions, type BarcodeScanningResult } from "expo-camera";
import { useRef, useState } from "react";
import { Linking, Modal, StyleSheet, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { linkErrorText, parsePairingLink, type PairingLink } from "@/connect/pairingLink";
import { inBackground } from "@/log";
import { useTheme } from "@/theme";
import { Button, Icon, Text } from "@/ui";

/**
 * The app's own QR scanner for pairing links (ADR 0007, section 1): the code is read here, never handed to the system,
 * so no other app can be given it. A QR code that isn't a pairing link is said so and scanning goes on.
 */
export function Scanner({ visible, onLink, onClose }: { visible: boolean; onLink: (link: PairingLink) => void; onClose: () => void }) {
  return (
    <Modal visible={visible} animationType="slide" presentationStyle="fullScreen" onRequestClose={onClose}>
      {visible ? <ScannerBody onLink={onLink} onClose={onClose} /> : null}
    </Modal>
  );
}

function ScannerBody({ onLink, onClose }: { onLink: (link: PairingLink) => void; onClose: () => void }) {
  const { colors, spacing } = useTheme();
  const [permission, ask] = useCameraPermissions();
  const [problem, setProblem] = useState<string | null>(null);
  // The camera reports the same code many times a second: one link is taken once, one bad code said once.
  const done = useRef(false);
  const lastBad = useRef<string | null>(null);

  const scanned = ({ data }: BarcodeScanningResult) => {
    if (done.current || data === lastBad.current) return;
    const parsed = parsePairingLink(data.trim());
    if (parsed.ok) {
      done.current = true;
      onLink(parsed.link);
      return;
    }
    lastBad.current = data;
    setProblem(linkErrorText(parsed.error));
  };

  return (
    <SafeAreaView style={[styles.fill, { backgroundColor: colors.background }]} edges={["top", "bottom"]}>
      <View style={{ padding: spacing.lg, gap: spacing.sm }}>
        <Text variant="title" accessibilityRole="header">
          Scan a pairing code
        </Text>
        <Text tone="secondary">
          Point the camera at the QR code your server or another signed-in device shows: Settings, Pair another device.
        </Text>
      </View>
      <View style={[styles.fill, { marginHorizontal: spacing.lg, borderRadius: 16, overflow: "hidden", backgroundColor: colors.surface }]}>
        {permission?.granted ? (
          <CameraView style={styles.fill} facing="back" barcodeScannerSettings={{ barcodeTypes: ["qr"] }} onBarcodeScanned={scanned} />
        ) : (
          <View style={[styles.fill, styles.centre, { padding: spacing.xl, gap: spacing.md }]}>
            <Icon name="qr-code-outline" size={44} color={colors.accent} />
            <Text align="center">Emberhut uses the camera only to read the pairing QR code. Nothing is recorded.</Text>
            {permission === null || permission.canAskAgain ? (
              <Button label="Allow the camera" onPress={() => inBackground(ask(), "Scanner: ask for the camera")} />
            ) : (
              <>
                <Text align="center" tone="secondary">
                  The camera is off for Emberhut. Turn it on in the system settings, or paste the link instead.
                </Text>
                <Button
                  label="Open settings"
                  variant="secondary"
                  onPress={() => inBackground(Linking.openSettings(), "Scanner: open settings")}
                />
              </>
            )}
          </View>
        )}
      </View>
      <View style={{ padding: spacing.lg, gap: spacing.md }}>
        {problem ? (
          <Text variant="callout" tone="danger" accessibilityLiveRegion="polite">
            {problem}
          </Text>
        ) : null}
        <Button label="Cancel" variant="secondary" onPress={onClose} block />
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({ fill: { flex: 1 }, centre: { alignItems: "center", justifyContent: "center" } });
