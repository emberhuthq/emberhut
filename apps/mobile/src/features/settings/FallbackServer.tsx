import { useNavigation } from "@react-navigation/native";
import { Alert } from "react-native";
import { dialect } from "@/api";
import { inBackground } from "@/log";
import { fallbackStore, sessionStore } from "@/services";
import { useFallback, useSession } from "@/state/hooks";
import { ListRow, Section, Toggle } from "@/ui";
import { hostOf } from "./servers";

/**
 * The fallback server: the one the app goes to by itself when this phone's server stops answering, and leaves only when
 * Try again finds the server answering. Set up by pairing: a new server, keeping this one as the fallback, or a fallback
 * beside this one. In a build with a wire dialect, which of the two speaks it.
 */
export function FallbackServer() {
  const navigation = useNavigation();
  const fallback = useSession((s) => s.fallback);
  const speaksDialect = useSession((s) => s.speaksDialect);
  const using = useFallback((s) => s.using);
  const checking = useFallback((s) => s.checking);
  const stillDown = useFallback((s) => s.stillDown);
  const setDialect = (slot: "server" | "fallback", on: boolean) =>
    inBackground(sessionStore.getState().setSpeaksDialect(slot, on), "FallbackServer: setSpeaksDialect");
  const remove = () =>
    Alert.alert("Remove the fallback server?", "This phone signs out of it. When the server stops answering, the app has nowhere to go.", [
      { text: "Cancel", style: "cancel" },
      {
        text: "Remove",
        style: "destructive",
        onPress: () => inBackground(sessionStore.getState().removeFallback(), "FallbackServer: removeFallback"),
      },
    ]);
  const status = !fallback
    ? undefined
    : !fallback.signedIn
      ? "It signed this phone out: pair with it again"
      : using === "fallback"
        ? stillDown
          ? "In use: the server still isn't answering"
          : "In use: the server isn't answering"
        : "Standing by";
  return (
    <Section
      title="Fallback server"
      footer="When this phone's server stops answering, the app goes to the fallback server by itself, and back only when you tap Try again."
    >
      {fallback ? <ListRow key="fallback" icon="swap-horizontal" title={hostOf(fallback.url)} subtitle={status} /> : null}
      {fallback && using === "fallback" ? (
        <ListRow
          key="try"
          icon="refresh"
          title={checking ? "Asking the server…" : "Try the server again"}
          onPress={checking ? undefined : () => inBackground(fallbackStore.getState().tryAgain(), "FallbackServer: tryAgain")}
          opens={false}
        />
      ) : null}
      {!dialect.identity ? (
        <ListRow
          key="server-dialect"
          icon="language-outline"
          title="The server speaks this build's dialect"
          trailing={
            <Toggle value={speaksDialect} onChange={(on) => setDialect("server", on)} label="The server speaks this build's dialect" />
          }
        />
      ) : null}
      {fallback && !dialect.identity ? (
        <ListRow
          key="fallback-dialect"
          icon="language-outline"
          title="The fallback server speaks it"
          trailing={
            <Toggle
              value={fallback.speaksDialect}
              onChange={(on) => setDialect("fallback", on)}
              label="The fallback server speaks this build's dialect"
            />
          }
        />
      ) : null}
      <ListRow
        key="switch"
        icon="arrow-forward-circle-outline"
        title="Move to a new server"
        subtitle="Pair with it; this one becomes the fallback server"
        onPress={() => navigation.navigate("AddServer", { role: "switch" })}
      />
      {!fallback || !fallback.signedIn ? (
        <ListRow
          key="add"
          icon="add-circle-outline"
          title={fallback ? "Pair with the fallback server again" : "Add a fallback server"}
          onPress={() => navigation.navigate("AddServer", { role: "fallback" })}
        />
      ) : null}
      {fallback ? (
        <ListRow key="remove" icon="trash-outline" title="Remove the fallback server" destructive onPress={remove} opens={false} />
      ) : null}
    </Section>
  );
}
