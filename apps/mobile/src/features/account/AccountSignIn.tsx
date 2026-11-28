import { useSignIn } from "@clerk/expo";
import { useState } from "react";
import { View } from "react-native";
import { accountConfig } from "@/config/account";
import { inBackground } from "@/log";
import { useTheme } from "@/theme";
import { Button, Section, Text, TextField } from "@/ui";

/** What Clerk's refusal says, in its own words (its long message is the sentence meant for people). */
const clerkText = (error: { message: string; longMessage?: string | undefined }): string => error.longMessage ?? error.message;

/**
 * Signing in to the emberhut.com account: a code Clerk emails, or a passkey when the build has them. Clerk's own screens
 * aren't used; these are the app's. Signing in lets the phone list your machines, and nothing more.
 */
export function AccountSignIn() {
  const { spacing } = useTheme();
  const { signIn, fetchStatus } = useSignIn();
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const busy = fetchStatus === "fetching";

  /** Makes the signed-in session the active one, or says why it isn't done. */
  const finish = async () => {
    if (signIn.status !== "complete") {
      setError("This account asks for another step to sign in, which the app doesn't offer. Sign in on the web, then try again.");
      return;
    }
    const { error: failed } = await signIn.finalize();
    if (failed) setError(clerkText(failed));
  };
  const sendCode = async () => {
    setError(null);
    const { error: failed } = await signIn.emailCode.sendCode({ emailAddress: email.trim() });
    if (failed) setError(clerkText(failed));
    else setSent(true);
  };
  const verify = async () => {
    setError(null);
    const { error: failed } = await signIn.emailCode.verifyCode({ code: code.trim() });
    if (failed) setError(clerkText(failed));
    else await finish();
  };
  const passkey = async () => {
    setError(null);
    const { error: failed } = await signIn.passkey({ flow: "discoverable" });
    if (failed) setError(clerkText(failed));
    else await finish();
  };
  const startOver = () => {
    inBackground(signIn.reset(), "AccountSignIn: reset");
    setSent(false);
    setCode("");
    setError(null);
  };

  return (
    <Section
      title="Sign in to your account"
      footer="Your account lists your huts on each device you sign in on. It never lets anyone into one: pairing still takes a code from the hut."
    >
      <View key="form" style={{ padding: spacing.lg, gap: spacing.md }}>
        {sent ? (
          <>
            <Text variant="callout">We emailed a code to {email.trim()}. It works for 10 minutes.</Text>
            <TextField
              value={code}
              onChangeText={setCode}
              placeholder="123456"
              keyboardType="number-pad"
              autoComplete="one-time-code"
              textContentType="oneTimeCode"
              returnKeyType="go"
              onSubmitEditing={() => inBackground(verify(), "AccountSignIn: verify")}
              accessibilityLabel="The code from the email"
            />
            <Button
              label="Sign in"
              onPress={() => inBackground(verify(), "AccountSignIn: verify")}
              disabled={code.trim() === ""}
              loading={busy}
              block
            />
            <Button label="Use another email" variant="plain" onPress={startOver} />
          </>
        ) : (
          <>
            <TextField
              value={email}
              onChangeText={setEmail}
              placeholder="you@example.com"
              keyboardType="email-address"
              autoCapitalize="none"
              autoCorrect={false}
              autoComplete="email"
              textContentType="emailAddress"
              returnKeyType="send"
              onSubmitEditing={() => inBackground(sendCode(), "AccountSignIn: send code")}
              accessibilityLabel="Your account's email"
            />
            <Button
              label="Email me a code"
              onPress={() => inBackground(sendCode(), "AccountSignIn: send code")}
              disabled={!email.includes("@")}
              loading={busy}
              block
            />
            {accountConfig?.passkeys ? (
              <Button
                label="Sign in with a passkey"
                icon="finger-print-outline"
                variant="secondary"
                onPress={() => inBackground(passkey(), "AccountSignIn: passkey")}
                disabled={busy}
                block
              />
            ) : null}
          </>
        )}
        {error ? (
          <Text variant="callout" tone="danger" accessibilityLiveRegion="polite">
            {error}
          </Text>
        ) : null}
      </View>
    </Section>
  );
}
