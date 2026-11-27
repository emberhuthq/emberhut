import { useMutation, useQueryClient } from "@tanstack/react-query";
import * as Clipboard from "expo-clipboard";
import { Alert, View } from "react-native";
import type { AuthInfo, SignedInDevice } from "@/api";
import { parsePairingLink } from "@/connect/pairingLink";
import { parseInstant, relative } from "@/format/time";
import { inBackground } from "@/log";
import { api, sessionStore } from "@/services";
import { useSession } from "@/state/hooks";
import { queryKeys } from "@/state/query";
import { useTheme } from "@/theme";
import { Button, ListRow, QrCode, Section, Text } from "@/ui";

/** Your signed-in devices, this one marked; any of them signs out from here (this one too, then the app asks to pair). */
export function Devices({ auth }: { auth: AuthInfo | null | undefined }) {
  const client = useQueryClient();
  const self = useSession((s) => s.device);
  const revoke = useMutation({
    mutationFn: (device: number) => api.devices.revoke(device),
    onSettled: () => client.invalidateQueries({ queryKey: queryKeys.allStates }),
  });
  if (!auth) return null;
  const signOut = (device: SignedInDevice) => {
    const mine = device.id === self;
    Alert.alert(
      mine ? "Sign out this phone?" : `Sign out ${device.name ?? "this device"}?`,
      mine ? "You'll need a new pairing code to sign in again." : "It will need to sign in again to use Emberhut.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Sign out",
          style: "destructive",
          onPress: () => (mine ? inBackground(sessionStore.getState().signOut(), "Devices: sign out") : revoke.mutate(device.id)),
        },
      ],
    );
  };
  return (
    <Section
      title="Devices"
      footer={
        auth.mode === "enforce"
          ? "Emberhut lets in only signed-in devices."
          : "Emberhut doesn't require sign-in yet: devices without it still get in."
      }
    >
      {auth.devices.map((device) => {
        const seen = parseInstant(device.seen);
        return (
          <ListRow
            key={device.id}
            icon={device.id === self ? "phone-portrait" : "laptop-outline"}
            title={`${device.name ?? "A device"}${device.id === self ? " · this one" : ""}`}
            subtitle={[device.model, seen ? `last seen ${relative(seen)}` : null].filter(Boolean).join(" · ") || undefined}
            trailing={
              <Button
                label="Sign out"
                variant="plain"
                onPress={() => signOut(device)}
                accessibilityLabel={`Sign out ${device.name ?? "this device"}`}
              />
            }
          />
        );
      })}
      {revoke.isError ? <ListRow key="error" title={`Not signed out: ${revoke.error.message}`} destructive /> : null}
    </Section>
  );
}

/**
 * A one-time code for a new device (an Android phone, an emulator), from this signed-in one; with its pairing link as a
 * QR code when the server makes one, for the other device's scanner.
 */
export function PairAnother() {
  const { colors, spacing } = useTheme();
  const code = useMutation({ mutationFn: () => api.devices.newPairingCode() });
  const expires = parseInstant(code.data?.expires);
  // Shown only as the link the server made, and only when it reads as one: the other device reads it the same way.
  const link = code.data?.link !== undefined && parsePairingLink(code.data.link).ok ? code.data.link : null;
  return (
    <Section
      title="Pair another device"
      footer={
        link
          ? "On the other device's sign-in screen, Scan a QR code and scan this one, or type the code. It works once, for 10 minutes."
          : "Type the code in the other device's Emberhut app. It works once, for 10 minutes."
      }
    >
      <View key="pair" style={{ padding: spacing.lg, gap: spacing.md, backgroundColor: colors.surface }}>
        {code.data ? (
          <View
            accessible
            accessibilityLabel={`Pairing code ${code.data.code.split("").join(" ")}`}
            style={{ alignItems: "center", gap: spacing.xs }}
          >
            <Text
              selectable
              style={{
                fontSize: 34,
                lineHeight: 42,
                fontWeight: "700",
                letterSpacing: 4,
                fontVariant: ["tabular-nums"],
                color: colors.text,
              }}
            >
              {code.data.code}
            </Text>
            {expires ? (
              <Text variant="footnote" tone="secondary">
                Works once, until {expires.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" })}
              </Text>
            ) : null}
          </View>
        ) : null}
        {link ? (
          <View key="qr" style={{ alignItems: "center", gap: spacing.sm }}>
            <QrCode value={link} size={220} label="Pairing QR code. Scan it with Emberhut on the other device." />
            <Button
              label="Copy the pairing link"
              icon="copy-outline"
              variant="plain"
              onPress={() => inBackground(Clipboard.setStringAsync(link), "PairAnother: copy link")}
            />
          </View>
        ) : null}
        {code.isError ? (
          <Text variant="footnote" tone="danger">
            No code: {code.error.message}
          </Text>
        ) : null}
        <Button
          label={code.data ? "Make another code" : "Make a pairing code"}
          icon="key-outline"
          variant="secondary"
          onPress={() => code.mutate()}
          loading={code.isPending}
        />
      </View>
    </Section>
  );
}
