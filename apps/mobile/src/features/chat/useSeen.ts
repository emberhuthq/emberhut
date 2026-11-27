import { useIsFocused } from "@react-navigation/native";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { AppState } from "react-native";
import { api } from "@/services";
import { queryKeys } from "@/state/query";

/**
 * Emberhut's answers you have on screen are seen: once when the room opens with unread ones, and again for each new answer
 * that arrives while you're looking. The room's badge then clears on your other devices too.
 */
export function useSeen(room: number, unread: boolean, latest: number | undefined) {
  const client = useQueryClient();
  const focused = useIsFocused();
  const marked = useRef<string | null>(null);
  useEffect(() => {
    if (!focused || !unread || AppState.currentState !== "active") return;
    const key = `${room}:${latest ?? 0}`;
    if (marked.current === key) return;
    marked.current = key;
    api.chat
      .seen(room)
      .then(() => client.invalidateQueries({ queryKey: queryKeys.state(room) }))
      .catch(() => {
        marked.current = null;
      });
  }, [client, focused, room, unread, latest]);
}
