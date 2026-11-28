import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { AppState } from "react-native";
import { ApiError, followRoomLive, type ApprovalsQuery, type EmberhutState, type SettingsChange, type TasksQuery } from "@/api";
import { inBackground } from "@/log";
import { api, chatStore, connectionStore, liveStore, themeStore } from "@/services";
import { usePrefs, useServerInUse, useSession } from "./hooks";
import { queryKeys } from "./query";

const pollMs = 3_000;
/** While failing: quiet retries a second apart. */
const retryMs = 1_000;

/**
 * A room's state, polled every 3 s while the app is in front; its chat joins the room's held chat. Without a room, the
 * room on screen, followed as it changes: every page and the tab badge then read the one same state.
 */
export function useEmberhutState(asked?: number) {
  const onScreen = usePrefs((s) => s.room);
  const room = asked ?? onScreen;
  // Nothing to ask before secure storage is read, or before there is a server at all.
  const ready = useSession((s) => s.status !== "starting" && s.serverUrl !== null);
  return useQuery<EmberhutState>({
    queryKey: queryKeys.state(room),
    enabled: ready,
    retry: false,
    refetchInterval: (query) => (query.state.status === "error" ? retryMs : pollMs),
    queryFn: async ({ signal }): Promise<EmberhutState> => {
      try {
        const state = await api.state.get({ room, looking: AppState.currentState === "active", signal });
        connectionStore.getState().noteSuccess();
        chatStore.getState().hold(room, state.chat);
        themeStore.getState().noteState(state.theme);
        return state;
      } catch (cause) {
        if (!signal.aborted)
          connectionStore.getState().noteFailure(cause instanceof ApiError && !cause.unreachable ? cause.message : "Can't reach Emberhut");
        throw cause;
      }
    },
  });
}

/** One task's page; a live one asks again every 4 s. */
export function useTask(id: number, live = false) {
  return useQuery({
    queryKey: queryKeys.task(id),
    queryFn: ({ signal }) => api.tasks.get(id, signal),
    refetchInterval: live ? 4_000 : false,
  });
}

/** Tasks from the server's whole list; `enabled` false asks nothing yet (a search not typed, a fold not opened). */
export function useTasks(query: TasksQuery, enabled = true) {
  return useQuery({ queryKey: queryKeys.tasks(query), queryFn: () => api.tasks.list(query), enabled });
}

export function useFacts() {
  return useQuery({ queryKey: queryKeys.facts, queryFn: () => api.settings.facts() });
}

export function useSwitches() {
  return useQuery({ queryKey: queryKeys.switches, queryFn: () => api.settings.switches() });
}

/** The model provider saved for the agent; `enabled` false asks nothing (not signed in). */
export function useAgentProvider(enabled = true) {
  return useQuery({ queryKey: queryKeys.agentProvider, queryFn: ({ signal }) => api.agentProvider.provider(signal), enabled });
}

export function useEnvironments() {
  return useQuery({ queryKey: queryKeys.environments, queryFn: () => api.settings.environments(), refetchInterval: 10_000 });
}

const notThere = (error: unknown) => error instanceof ApiError && error.status === 404;

/**
 * Each runner machine's live capacity, every 10 s while `polling` (its screen in front). A 404 (a server on older code)
 * isn't asked again until the screen opens again.
 */
export function useMachines(polling: boolean) {
  return useQuery({
    queryKey: queryKeys.machines,
    queryFn: ({ signal }) => api.machines.list(signal),
    refetchInterval: (query) => (polling && !notThere(query.state.error) ? 10_000 : false),
    retry: (failures, error) => !notThere(error) && failures < 2,
  });
}

/**
 * The machines without their day of history (`?history=0`): their names and whether they take new work. Names hardly
 * change, so by default they're asked again only after 5 minutes; New session asks for a fresh one.
 */
export function useMachineSummary(staleTime = 5 * 60_000) {
  return useQuery({
    queryKey: queryKeys.machineSummary,
    queryFn: ({ signal }) => api.machines.list(signal, { history: false }),
    staleTime,
    retry: (failures, error) => !notThere(error) && failures < 2,
  });
}

/**
 * Your parties, the presets and the models; every 5 s while `polling` (the list in front), for the running ones' last lines.
 * A 404 (a server on older code) isn't asked again until the screen opens again.
 */
export function useParties(polling = false) {
  return useQuery({
    queryKey: queryKeys.parties,
    queryFn: ({ signal }) => api.parties.list(signal),
    refetchInterval: (query) => (polling && !notThere(query.state.error) ? 5_000 : false),
    retry: (failures, error) => !notThere(error) && failures < 2,
  });
}

/**
 * Approval requests (GET /api/approvals), every 10 s while `polling` (the list in front) and at once on the live stream's
 * `approvals` event. A 404 (a server without agents) isn't asked again until the screen opens again.
 */
export function useApprovals(query: ApprovalsQuery, polling = false) {
  return useQuery({
    queryKey: queryKeys.approvals(query),
    queryFn: ({ signal }) => api.approvals.list(query, signal),
    refetchInterval: (q) => (polling && !notThere(q.state.error) ? 10_000 : false),
    retry: (failures, error) => !notThere(error) && failures < 2,
  });
}

/** One approval request as it is now (GET /api/approvals/{id}). */
export function useApproval(id: number) {
  return useQuery({
    queryKey: queryKeys.approval(id),
    queryFn: ({ signal }) => api.approvals.get(id, signal),
    retry: (failures, error) => !notThere(error) && failures < 2,
  });
}

/** Every agent with its settings (GET /api/agents): a payment's currency is its agent's. Asked again after a minute. */
export function useAgents(enabled = true) {
  return useQuery({
    queryKey: queryKeys.agents,
    queryFn: ({ signal }) => api.agents.list(signal),
    staleTime: 60_000,
    enabled,
    retry: (failures, error) => !notThere(error) && failures < 2,
  });
}

/** A change through POST /api/settings; the states refresh after it. */
export function useApplySettings() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (change: SettingsChange) => api.settings.apply(change),
    onSettled: () => client.invalidateQueries({ queryKey: queryKeys.allStates }),
  });
}

/**
 * Moves a session to another task (a server that keeps plans). Settled, the states and every task page are fetched
 * again: the session leaves the page it was on and joins the new one's.
 */
export function useMoveSession() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ session, task }: { session: string; task: number }) => api.sessions.moveToTask(session, task),
    onSettled: async () => {
      await Promise.all([
        client.invalidateQueries({ queryKey: queryKeys.allStates }),
        client.invalidateQueries({ queryKey: queryKeys.allTasks }),
      ]);
    },
  });
}

/**
 * Follows a room's live line while mounted. When a turn ends (null after a line), the answer is already in the server's
 * chat: the state is fetched again so the answer and the end of the working sign land together. An approval request's
 * change (the stream's `approvals` event) fetches the states and the approvals pages again.
 */
export function useRoomLive(room: number) {
  const client = useQueryClient();
  const ready = useSession((s) => s.status !== "starting" && s.serverUrl !== null);
  const server = useServerInUse();
  useEffect(() => {
    if (!ready) return;
    const controller = new AbortController();
    inBackground(
      followRoomLive(
        api.http,
        room,
        (live) => {
          const before = liveStore.getState().byRoom[room] ?? null;
          if (live === null && before !== null)
            inBackground(client.invalidateQueries({ queryKey: queryKeys.state(room) }), "queries: invalidateQueries");
          liveStore.getState().set(room, live);
        },
        {
          signal: controller.signal,
          onConnection: (connected) => liveStore.getState().setFollowing(room, connected),
          // A request changed: the states carry the pending ones and the counts, the approvals pages the rest.
          onApprovals: () => {
            inBackground(client.invalidateQueries({ queryKey: queryKeys.allStates }), "queries: invalidateQueries");
            inBackground(client.invalidateQueries({ queryKey: queryKeys.allApprovals }), "queries: invalidateQueries");
          },
        },
      ),
      "useRoomLive: follow",
    );
    return () => controller.abort();
  }, [client, room, ready, server]);
}
