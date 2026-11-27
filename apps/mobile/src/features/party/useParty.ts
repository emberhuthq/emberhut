import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useReducer, useState } from "react";
import { followParty, type Party, type PartyLiveEvent, type PartyPage } from "@/api";
import { inBackground } from "@/log";
import { api } from "@/services";
import { useServerInUse } from "@/state/hooks";
import { queryKeys } from "@/state/query";
import { applyPartyEvent, emptyParty, loadedParty, type PartyView } from "./party";

type Action = { type: "loaded"; page: PartyPage } | { type: "event"; event: PartyLiveEvent } | { type: "party"; party: Party };

const reduce = (v: PartyView, a: Action): PartyView =>
  a.type === "loaded" ? loadedParty(a.page) : a.type === "party" ? { ...v, party: a.party } : applyPartyEvent(v, a.event);

/**
 * One party: its talk so far, then its live stream (a snapshot on each connection, then each change), reconnecting after
 * drops. `act` sends one of your controls and takes the party it answers with.
 */
export function useParty(id: number) {
  const client = useQueryClient();
  const [view, dispatch] = useReducer(reduce, emptyParty);
  const [ready, setReady] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [dropped, setDropped] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const server = useServerInUse();

  useEffect(() => {
    const controller = new AbortController();
    api.parties
      .get(id, controller.signal)
      .then((page) => {
        if (controller.signal.aborted) return;
        dispatch({ type: "loaded", page });
        setReady(true);
        inBackground(
          followParty(api.http, id, (event) => dispatch({ type: "event", event }), {
            signal: controller.signal,
            onConnection: (connected) => setDropped(!connected),
          }),
          "useParty: follow",
        );
      })
      .catch((cause: unknown) => !controller.signal.aborted && setFailed(cause instanceof Error ? cause.message : "unknown"));
    return () => controller.abort();
  }, [id, attempt, server]);

  const retry = useCallback(() => {
    setFailed(null);
    setAttempt((n) => n + 1);
  }, []);

  const act = useCallback(
    async (send: () => Promise<{ party: Party }>): Promise<boolean> => {
      try {
        const reply = await send();
        dispatch({ type: "party", party: reply.party });
        setProblem(null);
        inBackground(client.invalidateQueries({ queryKey: queryKeys.parties }), "useParty: invalidateQueries");
        return true;
      } catch (cause) {
        setProblem(cause instanceof Error ? cause.message : "Couldn't reach Emberhut");
        return false;
      }
    },
    [client],
  );

  return { view, ready, failed, retry, problem: problem ?? (dropped ? "Reconnecting…" : null), act };
}
