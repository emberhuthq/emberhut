import { useAuth, useUser } from "@clerk/expo";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo } from "react";
import { Alert } from "react-native";
import { DirectoryError, type DirectoryMachine } from "@/account/directory";
import { relative } from "@/format/time";
import { inBackground } from "@/log";
import { accountDirectory, sessionStore } from "@/services";
import { useSession } from "@/state/hooks";
import { queryKeys } from "@/state/query";
import { needsSignIn } from "@/state/session";
import { Button, ErrorState, ListRow, Loading, Screen, Section } from "@/ui";
import { AccountSignIn } from "./AccountSignIn";

/** How often the list is asked for again while it is on screen: whether each machine is connected changes. */
const refreshMs = 30_000;

/**
 * The emberhut.com account (ADR 0007, section 5): sign in, see the account's machines and whether each is connected, unlink
 * one, sign out. Only in builds with the account on. Tapping a machine starts the normal pairing with it pinned: the
 * account finds the machine, and the machine still decides who gets in.
 */
export function AccountScreen() {
  const { isLoaded, isSignedIn } = useAuth();
  if (!isLoaded) return <Loading label="Reading your account…" />;
  return <Screen scroll>{isSignedIn ? <AccountMachines /> : <AccountSignIn />}</Screen>;
}

function AccountMachines() {
  const { getToken, signOut } = useAuth();
  const { user } = useUser();
  const client = useQueryClient();
  const navigation = useNavigation();
  const focused = useIsFocused();
  const paired = useSession((s) => s.machineId);
  const directory = useMemo(() => accountDirectory(() => getToken({ template: "emberhut-directory" })), [getToken]);
  const machines = useQuery({
    queryKey: queryKeys.accountMachines,
    queryFn: () => directory!.machines(),
    enabled: directory !== null,
    // After a 429 or 503, nothing is asked again until its Retry-After has passed.
    refetchInterval: (query) => (focused ? Math.max(refreshMs, waitLeft(query.state.error, query.state.errorUpdatedAt)) : false),
    // Asking again at once only helps when nothing answered; a refusal or a busy directory says so, and its retry waits.
    retry: (failures, error) => error instanceof DirectoryError && error.kind === "unreachable" && failures < 2,
  });
  const unlink = useMutation({
    mutationFn: (id: string) => directory!.unlink(id),
    onSettled: () => client.invalidateQueries({ queryKey: queryKeys.accountMachines }),
  });
  const leave = useCallback(
    () =>
      inBackground(
        signOut().then(() => client.removeQueries({ queryKey: queryKeys.accountMachines })),
        "AccountScreen: sign out",
      ),
    [signOut, client],
  );

  // The directory says the account's sign-in is over: sign out of the account (never of a machine). A busy directory
  // (429, 503) is only "try again later".
  const signedOutError = [machines.error, unlink.error].some((e) => e instanceof DirectoryError && e.kind === "signIn");
  useEffect(() => {
    if (signedOutError) leave();
  }, [signedOutError, leave]);

  const confirmSignOut = () =>
    Alert.alert("Sign out of your account?", "This phone stops listing your machines. It stays paired with its server.", [
      { text: "Cancel", style: "cancel" },
      { text: "Sign out", style: "destructive", onPress: leave },
    ]);
  const confirmUnlink = (machine: DirectoryMachine) =>
    Alert.alert(
      `Unlink ${machine.name}?`,
      "It leaves your account and can't use the account's relay. Devices paired with it stay paired, on its other routes.",
      [
        { text: "Cancel", style: "cancel" },
        { text: "Unlink", style: "destructive", onPress: () => unlink.mutate(machine.id) },
      ],
    );
  const pairWith = (machine: DirectoryMachine) => {
    if (machine.id === paired) {
      Alert.alert(`This phone uses ${machine.name}`, "It is paired with it already.");
      return;
    }
    const session = sessionStore.getState();
    const pin = { id: machine.id, name: machine.name };
    if (session.status === "signedIn") {
      Alert.alert(
        `Pair with ${machine.name}?`,
        `This phone signs out of the server it uses now, then pairs with ${machine.name}, with a code or QR code from it.`,
        [
          { text: "Cancel", style: "cancel" },
          {
            text: "Sign out and pair",
            onPress: () => {
              session.pinMachine(pin);
              inBackground(session.signOut(), "AccountScreen: sign out to pair");
            },
          },
        ],
      );
      return;
    }
    // Signed out: pinning brings up the pairing screen (or goes back to it, when this screen was opened from there).
    const fromSignIn = needsSignIn(session);
    session.pinMachine(pin);
    if (fromSignIn) navigation.goBack();
  };

  const email = user?.primaryEmailAddress?.emailAddress;
  return (
    <>
      <Section title="Your machines" footer="Tap one to pair this phone with it: it still takes a code or QR code from that machine.">
        {machines.data?.length === 0 ? (
          <ListRow
            key="none"
            icon="desktop-outline"
            title="No machines yet"
            subtitle="Add one from the desktop app, or with emberhut-server link."
          />
        ) : null}
        {(machines.data ?? []).map((machine) => (
          <ListRow
            key={machine.id}
            icon={machine.connected ? "desktop" : "desktop-outline"}
            title={machine.name}
            subtitle={[seenText(machine), machine.id === paired ? "this phone uses it" : null].filter(Boolean).join(" · ")}
            onPress={() => pairWith(machine)}
            trailing={
              <Button
                label="Unlink"
                variant="plain"
                onPress={() => confirmUnlink(machine)}
                accessibilityLabel={`Unlink ${machine.name} from your account`}
              />
            }
          />
        ))}
        {machines.isPending ? <ListRow key="loading" icon="hourglass-outline" title="Asking your account…" /> : null}
        {unlink.isError && !signedOutError ? (
          <ListRow key="unlink-error" title={`Not unlinked: ${unlink.error.message}`} destructive />
        ) : null}
      </Section>
      {machines.isError && !signedOutError ? (
        <ErrorState message={machines.error.message} onRetry={() => inBackground(machines.refetch(), "AccountScreen: refetch")} />
      ) : null}
      <Section title="Account">
        <ListRow key="email" icon="person-circle-outline" title={email ?? "Signed in"} subtitle="Your emberhut.com account" />
      </Section>
      <Button label="Sign out of your account" variant="destructive" icon="log-out-outline" onPress={confirmSignOut} />
    </>
  );
}

/** How long a busy directory asked to be left alone, from when it said so; 0 when it didn't. */
function waitLeft(error: unknown, at: number): number {
  if (!(error instanceof DirectoryError) || error.kind !== "later" || error.retryAfter === undefined) return 0;
  return Math.max(0, at + error.retryAfter * 1000 - Date.now());
}

/** Connected now, or when it last was. */
function seenText(machine: DirectoryMachine): string {
  if (machine.connected) return "Connected";
  return machine.lastSeen === null ? "Never connected" : `Last seen ${relative(new Date(machine.lastSeen))}`;
}
