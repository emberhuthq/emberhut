import { accountConfig } from "@/config/account";
import { savedLine } from "@/features/ember/credentials";
import { usePrefs, useSession } from "@/state/hooks";
import { useAgentProvider, useEmberhutState, useEnvironments, useFacts, useSwitches } from "@/state/queries";
import { currentTheme } from "@/state/theme";
import { useTheme } from "@/theme";
import type { Now } from "./entries";

/** How the things Settings speaks of stand now, from what the app has fetched: Settings' front door and group screens read it. */
export function useSettingsNow(): Now {
  const signedIn = useSession((s) => s.status === "signedIn");
  const self = useSession((s) => s.device);
  const state = useEmberhutState();
  const provider = useAgentProvider(signedIn);
  const facts = useFacts();
  const switches = useSwitches();
  const environments = useEnvironments();
  const { scheme } = useTheme();
  const cache = usePrefs((s) => s.themeCache);
  const local = usePrefs((s) => s.theme);
  const showAgents = usePrefs((s) => s.showAgents);
  const agentsPlace = usePrefs((s) => s.agentsPlace);
  const appearance = currentTheme({ themeCache: cache, theme: local }).choice.appearance;
  const devices = state.data?.auth?.devices;
  const sources = (environments.data?.groups ?? []).flatMap((g) => g.items).filter((e) => e.group === "connections");
  return {
    onDuty: state.data?.on ?? false,
    working: (state.data?.on ?? false) && (state.data?.busy ?? false),
    jobs: (state.data?.tasks ?? []).filter((job) => job.status === "running").length,
    provider: provider.data ? savedLine(provider.data) : undefined,
    facts: facts.data?.facts.length,
    lessons: facts.data?.trials?.length ?? 0,
    dark: scheme === "dark",
    appearance: appearance === "light" || appearance === "dark" ? appearance : "system",
    devices: devices?.length,
    otherDevices: (devices ?? []).some((d) => d.id !== self),
    signedIn,
    failingTools: (switches.data?.mcp ?? []).filter((m) => m.on && m.state === "failed").map((m) => m.name),
    sourcesToFix: sources.filter((e) => e.state === "warn" || e.state === "down").map((e) => e.name),
    sourcesOn: sources.filter((e) => e.state === "ok").length,
    hasAccount: accountConfig !== null,
    showAgents,
    agentsPlace,
  };
}
