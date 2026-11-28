import { useNavigation } from "@react-navigation/native";
import * as Clipboard from "expo-clipboard";
import { useEffect, useState } from "react";
import { ScrollView, StyleSheet, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { checkServerAddress } from "@/api";
import { accountConfig } from "@/config/account";
import { linkErrorText, pairingLinkBase, parsePairingLink, type PairingLink } from "@/connect/pairingLink";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { sessionStore } from "@/services";
import type { PairRole } from "@/state/session";
import { useSession } from "@/state/hooks";
import { useTheme } from "@/theme";
import { Button, Card, Icon, KeyboardAvoider, Text, TextField } from "@/ui";
import { codeLength, formatCode, isComplete } from "./code";
import { Scanner } from "./Scanner";

const unreachable = "Can't reach Emberhut";

/**
 * Pairing this phone with its server: the server's address and a one-time code from it, or its pairing link, scanned
 * with the app's own scanner or pasted. A phone the server won't let in (not on its network) is told so first, with how to
 * fix it; while the server doesn't require sign-in yet, you can say not now. With the account on, a machine picked from it
 * is pinned: the pairing must prove it is that one.
 */
export function SignInScreen() {
  return <PairingScreen role="server" />;
}

/**
 * Pairing with another server while signed in (Settings, Fallback server): a new server for this phone, the one it had
 * kept as its fallback ("switch"), or a fallback server beside it. Done, it goes back to Settings.
 */
export function AddServerScreen({ route, navigation }: RootScreenProps<"AddServer">) {
  return <PairingScreen role={route.params.role} onPaired={() => navigation.goBack()} />;
}

const headings: Record<PairRole, { title: string; detail: string }> = {
  server: {
    title: "Sign in to Emberhut",
    detail: "Pair this phone with your server: scan its pairing QR code, or type its address and a one-time code from it.",
  },
  switch: {
    title: "Move to a new server",
    detail:
      "Pair with the new server: scan its pairing QR code, or type its address and a one-time code from it. The server this phone has now stays as its fallback server, signed in, for when the new one stops answering.",
  },
  fallback: {
    title: "Add a fallback server",
    detail:
      "Pair with the server this phone goes to when its own stops answering: scan its pairing QR code, or type its address and a one-time code from it.",
  },
};

function PairingScreen({ role, onPaired }: { role: PairRole; onPaired?: () => void }) {
  const { colors, spacing } = useTheme();
  const signIn = role === "server";
  const required = useSession((s) => s.required);
  const refused = useSession((s) => s.refused);
  const pairing = useSession((s) => s.pairing);
  const error = useSession((s) => s.pairError);
  const startError = useSession((s) => s.startError);
  const deviceName = useSession((s) => s.deviceName);
  const later = useSession((s) => s.signInLater);
  const kept = useSession((s) => s.serverUrl);
  const pin = useSession((s) => s.pin);
  const navigation = useNavigation();
  // Another server starts from an empty address; sign-in from the one kept.
  const start = signIn ? (kept ?? "") : "";
  const [server, setServer] = useState(start);
  const [code, setCode] = useState("");
  const [name, setName] = useState(deviceName);
  const [scanning, setScanning] = useState(false);
  const [linkError, setLinkError] = useState<string | null>(null);

  // What an earlier pairing said belongs to that one.
  useEffect(() => {
    if (!signIn) sessionStore.setState({ pairError: null });
  }, [signIn]);

  const ready = server.trim() !== "" && isComplete(code);
  const address = checkServerAddress(server);
  const paired = (ok: boolean) => {
    if (ok) onPaired?.();
  };
  const pair = () => inBackground(sessionStore.getState().pair(code, name, server, role).then(paired), "SignInScreen: pair");
  const pairLink = (link: PairingLink) => {
    setScanning(false);
    setLinkError(null);
    inBackground(sessionStore.getState().pairLink(link, name, role).then(paired), "SignInScreen: pair from a link");
  };
  /** A pairing link as text (pasted, or typed into the address): paired from at once, or why it can't be. */
  const readLink = (text: string) => {
    const parsed = parsePairingLink(text.trim());
    if (parsed.ok) pairLink(parsed.link);
    else setLinkError(linkErrorText(parsed.error));
  };
  const paste = async () => {
    const text = await Clipboard.getStringAsync();
    if (text.trim() === "") setLinkError("There's nothing to paste. Copy the pairing link first.");
    else readLink(text);
  };
  /**
   * The address field takes a pairing link too, typed or pasted: the text is always kept as it is typed, and once it
   * reads as a whole link the phone pairs from it. A link still being typed shows what it lacks.
   */
  const typeServer = (typed: string) => {
    setServer(typed);
    if (!typed.trim().startsWith(`${pairingLinkBase}#`)) {
      setLinkError(null);
      return;
    }
    const parsed = parsePairingLink(typed.trim());
    if (!parsed.ok) {
      setLinkError(linkErrorText(parsed.error));
      return;
    }
    setServer(start);
    pairLink(parsed.link);
  };
  return (
    <SafeAreaView style={[styles.fill, { backgroundColor: colors.background }]} edges={signIn ? ["top", "bottom"] : ["bottom"]}>
      <KeyboardAvoider>
        <ScrollView
          contentContainerStyle={{ padding: spacing.xl, gap: spacing.lg, flexGrow: 1, justifyContent: "center" }}
          keyboardShouldPersistTaps="handled"
        >
          <View style={{ alignItems: "center", gap: spacing.sm }}>
            <Icon name={signIn ? "key" : "swap-horizontal"} size={44} color={colors.accent} />
            <Text variant="title" align="center" accessibilityRole="header">
              {headings[role].title}
            </Text>
            <Text tone="secondary" align="center">
              {headings[role].detail}
            </Text>
          </View>
          {signIn && pin ? <Pinned name={pin.name} /> : null}
          {signIn && refused ? <OffNetwork /> : null}
          {signIn && startError ? <Unreadable reason={startError} /> : null}
          <View style={{ gap: spacing.sm }}>
            <Button label="Scan a QR code" icon="qr-code-outline" onPress={() => setScanning(true)} loading={pairing} block />
            <Button
              label="Paste a pairing link"
              icon="clipboard-outline"
              variant="secondary"
              onPress={() => inBackground(paste(), "SignInScreen: paste")}
              disabled={pairing}
              block
            />
            {linkError ? (
              <Text variant="callout" tone="danger" accessibilityLiveRegion="polite">
                {linkError}
              </Text>
            ) : null}
          </View>
          <View style={{ gap: spacing.sm }}>
            <Text variant="footnote" weight="600" tone="secondary">
              Server
            </Text>
            <TextField
              value={server}
              onChangeText={typeServer}
              placeholder="https://my-server:18990"
              autoCapitalize="none"
              autoCorrect={false}
              keyboardType="url"
              textContentType="URL"
              accessibilityLabel="Server address"
            />
            {address.ok && address.plain ? (
              <Text variant="footnote" tone="warning" accessibilityLiveRegion="polite">
                Not encrypted: plain http, so the code and the sign-in cross the network as they are. Only on your own network or VPN.
              </Text>
            ) : null}
            <Text variant="footnote" weight="600" tone="secondary">
              Pairing code
            </Text>
            <TextField
              value={code}
              onChangeText={(typed) => setCode(formatCode(typed))}
              placeholder="ABCD-EFGH"
              autoCapitalize="characters"
              autoCorrect={false}
              autoComplete="one-time-code"
              maxLength={codeLength + 1}
              returnKeyType="go"
              onSubmitEditing={ready ? pair : undefined}
              accessibilityLabel="Pairing code"
              style={{ fontSize: 26, lineHeight: 32, letterSpacing: 4, textAlign: "center", fontWeight: "700", minHeight: 56 }}
            />
            <Text variant="footnote" weight="600" tone="secondary">
              This phone’s name in Emberhut
            </Text>
            <TextField
              value={name}
              onChangeText={setName}
              placeholder={deviceName}
              accessibilityLabel="This phone's name in Emberhut"
              maxLength={60}
            />
            {error ? (
              <Text variant="callout" tone="danger" accessibilityLiveRegion="polite">
                {error === unreachable
                  ? `${unreachable} at ${server.trim()}. Is the address right, and is this phone on its network?`
                  : error}
              </Text>
            ) : null}
          </View>
          <Button label="Pair" variant="secondary" onPress={pair} disabled={!ready} loading={pairing} block />
          <Card>
            <Text variant="headline">Getting a code</Text>
            <Text variant="callout">
              On a device that’s already signed in: Settings, then Pair another device. Scan the QR code it shows, or type its code.
            </Text>
            <Text variant="callout">On the server: its pairing command shows one, with its QR code.</Text>
            <Text variant="footnote" tone="secondary">
              A code reads like ABCD-EFGH, works once and for 10 minutes.
            </Text>
          </Card>
          {signIn && accountConfig ? (
            <Button
              label="Find your huts in your account"
              icon="person-circle-outline"
              variant="plain"
              onPress={() => navigation.navigate("Account")}
            />
          ) : null}
          {!signIn || required || kept === null ? null : <Button label="Not now" variant="plain" onPress={later} />}
        </ScrollView>
      </KeyboardAvoider>
      <Scanner visible={scanning} onLink={pairLink} onClose={() => setScanning(false)} />
    </SafeAreaView>
  );
}

/** A machine picked from the account: only it may answer this pairing. */
function Pinned({ name }: { name: string }) {
  const { colors, spacing } = useTheme();
  return (
    <Card accent={colors.accent}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm }}>
        <Icon name="desktop-outline" size={22} color={colors.accent} />
        <Text variant="headline" style={{ flex: 1 }}>
          Pairing with {name}
        </Text>
      </View>
      <Text variant="callout">
        Scan its QR code or type a code from it. Your account found it; the pairing still needs a code from {name}, and must prove it is
        that hut.
      </Text>
      <Button label="Pair with any hut" variant="plain" onPress={() => sessionStore.getState().pinMachine(null)} />
    </Card>
  );
}

/** Secure storage failed at start: what was kept can't be read now. Reading again often works (a locked phone unlocked). */
function Unreadable({ reason }: { reason: string }) {
  const { colors } = useTheme();
  return (
    <Card accent={colors.warning}>
      <Text variant="callout" accessibilityLiveRegion="polite">
        {reason} Try again, or pair this phone again.
      </Text>
      <Button label="Try again" variant="secondary" onPress={() => inBackground(sessionStore.getState().start(), "SignInScreen: start")} />
    </Card>
  );
}

/** The server let nothing through: this phone isn't on its network as its owner. */
function OffNetwork() {
  const { colors, spacing } = useTheme();
  return (
    <Card accent={colors.warning}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm }}>
        <Icon name="shield-outline" size={22} color={colors.warning} />
        <Text variant="headline" style={{ flex: 1 }}>
          Not on the server’s network
        </Text>
      </View>
      <Text variant="callout">
        The server only lets in its owner’s devices on its own network. Connect this phone to it (the VPN the server uses, if any) as the
        server’s owner, then try again.
      </Text>
      <Button label="Try again" variant="secondary" onPress={() => inBackground(sessionStore.getState().start(), "SignInScreen: start")} />
    </Card>
  );
}

const styles = StyleSheet.create({ fill: { flex: 1 } });
