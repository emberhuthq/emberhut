import { useCallback, useEffect, useReducer, useState } from "react";
import { followSession, type SessionLiveEvent, type SessionTranscript } from "@/api";
import { inBackground } from "@/log";
import { api } from "@/services";
import { useServerInUse } from "@/state/hooks";
import { applyEvent, emptyView, loaded, type LiveView } from "./sessionLive";

type Action = { type: "loaded"; transcript: SessionTranscript } | { type: "event"; event: SessionLiveEvent };

const reduce = (v: LiveView, a: Action): LiveView => (a.type === "loaded" ? loaded(a.transcript) : applyEvent(v, a.event));

/**
 * A session's newest steps, then its live stream from the last one, reconnecting with the last step it has: nothing
 * missed, nothing polled. `agent` narrows it to one helper agent's own steps.
 */
export function useSessionLive(id: string, agent: string | undefined) {
  const [view, dispatch] = useReducer(reduce, emptyView);
  const [ready, setReady] = useState(false);
  const [connected, setConnected] = useState(false);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const server = useServerInUse();

  useEffect(() => {
    const controller = new AbortController();
    api.sessions
      .transcript(id, { after: 0, limit: agent ? 1000 : 400, agent })
      .then((transcript) => {
        if (controller.signal.aborted) return;
        dispatch({ type: "loaded", transcript });
        setReady(true);
        inBackground(
          followSession(api.http, id, { after: transcript.events.at(-1)?.seq ?? 0, agent }, (event) => dispatch({ type: "event", event }), {
            signal: controller.signal,
            onConnection: setConnected,
          }),
          "useSessionLive: follow",
        );
      })
      .catch(() => !controller.signal.aborted && setFailed(true));
    return () => controller.abort();
  }, [id, agent, attempt, server]);

  const retry = useCallback(() => {
    setFailed(false);
    setAttempt((n) => n + 1);
  }, []);

  return { view, ready, connected, failed, retry };
}
