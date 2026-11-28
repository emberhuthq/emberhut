import { usePreventRemove } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ScrollView } from "react-native";
import type { ChatFile, ClaudeAccount, Effort } from "@/api";
import { Composer } from "@/features/chat/Composer";
import { placeableMachines } from "@/features/machines/machineNames";
import { isOlderServer } from "@/features/machines/machines";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { api } from "@/services";
import { usePrefs } from "@/state/hooks";
import { useMachineSummary, useParties, useEmberhutState } from "@/state/queries";
import { queryKeys } from "@/state/query";
import { useTheme } from "@/theme";
import {
  ActionSheet,
  EmptyState,
  ErrorState,
  KeyboardAvoider,
  ListRow,
  Loading,
  Screen,
  Section,
  Segmented,
  Text,
  TextField,
  Toggle,
} from "@/ui";
import { FolderBrowser } from "./FolderBrowser";
import { accountLine, accountsFor, quickFolders, sessionFolder, sessionMachine, wantsBranches } from "./newSession";
import { effortName, efforts, modelName, sessionModels, shortPath, shortTitle } from "./sessionList";

type Picking = "machine" | "account" | "model" | "effort" | "branch" | null;

/** Up to this many machines show as a segmented control; more go in a sheet. */
const segmentedMachines = 3;

/** One of your own sessions: the machine, the folder (optionally a worktree), the account, model and effort, then your first message. */
export function NewSessionScreen({ navigation }: RootScreenProps<"NewSession">) {
  const { spacing } = useTheme();
  const room = usePrefs((s) => s.room);
  const state = useEmberhutState(room);
  // The machines are the server's (GET /api/machines), those it places work on, named as it names them.
  const machines = useMachineSummary(0);
  const offered = placeableMachines(machines.data);
  const [picked, setPicked] = useState<string | null>(null);
  const machine = sessionMachine(picked, offered) ?? "";
  const place = offered.find((m) => m.id === machine)?.name ?? machine;
  const [cwd, setCwd] = useState("");
  const [browsing, setBrowsing] = useState(false);
  const [worktree, setWorktree] = useState(false);
  const [existing, setExisting] = useState(false);
  const [branch, setBranch] = useState("");
  const [account, setAccount] = useState("");
  // The models are the server's (the parties page names them); the first is the default until one is picked.
  const models = sessionModels(useParties().data?.models);
  const [pickedModel, setModel] = useState<string | null>(null);
  const model = models.find((m) => m.id === pickedModel)?.id ?? models[0]?.id ?? "";
  const [effort, setEffort] = useState<Effort>("high");
  const [picking, setPicking] = useState<Picking>(null);
  const [text, setText] = useState("");
  const [opening, setOpening] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  const roots = useQuery({
    queryKey: queryKeys.folders(machine),
    queryFn: () => api.sessions.folders(machine, undefined),
    enabled: machine !== "",
  });
  const folder = sessionFolder(cwd, roots.data);
  const branches = useQuery({
    queryKey: queryKeys.branches(machine, folder),
    queryFn: () => api.sessions.branches(machine, folder),
    enabled: wantsBranches(worktree, existing, folder),
  });
  const accounts = accountsFor(state.data?.accounts ?? [], machine);
  const chosen: ClaudeAccount | undefined = accounts.find((a) => a.id === account);

  // Back while browsing folders leaves the browser, not the new session.
  usePreventRemove(browsing, () => setBrowsing(false));

  const switchMachine = (m: string) => {
    setPicked(m);
    setCwd("");
    setBranch("");
    setAccount("");
  };

  const open = async (first: string, files: ChatFile[]): Promise<boolean> => {
    setOpening(true);
    setProblem(null);
    try {
      const opened = await api.sessions.start({
        machine,
        cwd: folder,
        text: first,
        model,
        effort,
        files,
        ...(account ? { account } : {}),
        ...(worktree ? { worktree: branch.trim() ? { branch: branch.trim() } : {} } : {}),
      });
      navigation.replace("Session", { id: opened.id, title: shortTitle(opened.session.title) });
      return true;
    } catch (cause) {
      setProblem(`Not opened: ${cause instanceof Error ? cause.message : "unknown"}`);
      setOpening(false);
      return false;
    }
  };

  if (machine === "") {
    if (machines.isPending) return <Loading label="Asking the huts…" />;
    if (isOlderServer(machines.error) || machines.data)
      return <EmptyState icon="hardware-chip-outline" title="No hut to start on" message={noMachineText(machines.data !== undefined)} />;
    return (
      <ErrorState
        message={machines.error?.message ?? "Can't read the huts"}
        onRetry={() => inBackground(machines.refetch(), "NewSessionScreen: refetch")}
      />
    );
  }

  if (browsing)
    return (
      <FolderBrowser
        machine={machine}
        place={place}
        start={roots.data?.documents ?? "~/Documents"}
        onPick={(p) => {
          setCwd(p);
          setBrowsing(false);
        }}
        onCancel={() => setBrowsing(false)}
      />
    );

  const sheet: { title: string; actions: { label: string; onPress: () => void }[] } | null =
    picking === "machine"
      ? { title: "Hut", actions: offered.map((m) => ({ label: m.name, onPress: () => switchMachine(m.id) })) }
      : picking === "account"
        ? {
            title: "Claude account",
            actions: [
              { label: "Most room", onPress: () => setAccount("") },
              ...accounts.map((a) => ({ label: accountLine(a), onPress: () => setAccount(a.id) })),
            ],
          }
        : picking === "model"
          ? { title: "Model", actions: models.map((m) => ({ label: m.name, onPress: () => setModel(m.id) })) }
          : picking === "effort"
            ? { title: "Effort", actions: efforts.map((e) => ({ label: effortName(e), onPress: () => setEffort(e) })) }
            : picking === "branch"
              ? { title: "Branch", actions: (branches.data?.branches ?? []).map((b) => ({ label: b, onPress: () => setBranch(b) })) }
              : null;

  return (
    <Screen padded={false} edges={["bottom"]}>
      <KeyboardAvoider>
        <ScrollView contentContainerStyle={{ padding: spacing.lg, gap: spacing.lg }} keyboardShouldPersistTaps="handled">
          {offered.length <= segmentedMachines ? (
            <Segmented
              accessibilityLabel="Hut"
              options={offered.map((m) => ({ value: m.id, label: m.name }))}
              value={machine}
              onChange={switchMachine}
            />
          ) : (
            <ListRow icon="hardware-chip-outline" title={place} subtitle="Hut" onPress={() => setPicking("machine")} />
          )}
          <Section title="Folder">
            <ListRow
              icon="folder-open-outline"
              title={folder ? shortPath(folder) : roots.isError ? `The runner on ${place} didn't answer` : "Choose…"}
              subtitle="Browse or type a path"
              onPress={() => setBrowsing(true)}
            />
            {quickFolders(roots.data)
              .filter((f) => f.path !== folder)
              .map((f) => (
                <ListRow
                  key={f.path}
                  icon="folder-outline"
                  title={f.name}
                  subtitle={shortPath(f.path)}
                  onPress={() => setCwd(f.path)}
                  opens={false}
                />
              ))}
          </Section>
          <Section
            title="Git"
            footer={
              worktree ? "Its own checkout in a new git worktree; the folder's own stays as it is." : "It works in the folder itself."
            }
          >
            <Toggle label="Git worktree" value={worktree} onChange={setWorktree} />
            {worktree && (
              <>
                <Segmented
                  accessibilityLabel="Branch"
                  options={[
                    { value: "new", label: "New branch" },
                    { value: "existing", label: "Existing branch" },
                  ]}
                  value={existing ? "existing" : "new"}
                  onChange={(v) => {
                    setExisting(v === "existing");
                    setBranch("");
                  }}
                />
                {existing ? (
                  <ListRow icon="git-branch-outline" title={branch || "Choose a branch…"} onPress={() => setPicking("branch")} />
                ) : (
                  <TextField
                    value={branch}
                    onChangeText={setBranch}
                    placeholder="Branch name (optional)"
                    autoCapitalize="none"
                    autoCorrect={false}
                    accessibilityLabel="Branch name"
                  />
                )}
              </>
            )}
          </Section>
          <Section
            title="Claude"
            footer="Most room picks the account with the most left, and it moves to another by itself if that one runs out."
          >
            <ListRow
              icon="person-circle-outline"
              title={chosen ? accountLine(chosen) : "Most room"}
              subtitle="Account"
              onPress={() => setPicking("account")}
            />
            <ListRow
              icon="sparkles-outline"
              title={modelName(model, models) ?? model}
              subtitle="Model"
              onPress={() => setPicking("model")}
            />
            <ListRow icon="speedometer-outline" title={effortName(effort)} subtitle="Effort" onPress={() => setPicking("effort")} />
          </Section>
          {problem && <Text tone="danger">{problem}</Text>}
        </ScrollView>
        <Composer
          text={text}
          setText={setText}
          placeholder={opening ? "Opening…" : "What should it do?"}
          dictation={state.data?.voice === true}
          onSend={open}
          disabled={opening || folder === "" || (worktree && existing && branch === "")}
        />
      </KeyboardAvoider>
      <ActionSheet visible={sheet !== null} title={sheet?.title} actions={sheet?.actions ?? []} onClose={() => setPicking(null)} />
    </Screen>
  );
}

/** Why there's nothing to pick: the server lists no machine it places work on, or doesn't list machines at all yet. */
const noMachineText = (listed: boolean): string =>
  listed
    ? "None of the server's huts takes new sessions now. The Huts page says why."
    : "Emberhut's server is on older code: it doesn't list its huts, so a session can't be started here yet.";
