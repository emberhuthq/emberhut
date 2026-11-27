import { focusManager } from "@tanstack/react-query";
import { AppState } from "react-native";

/** Polls run only while the app is in front: React Native has no window focus events, so AppState stands in. */
export function followAppFocus(): void {
  focusManager.setEventListener((setFocused) => {
    const subscription = AppState.addEventListener("change", (state) => setFocused(state === "active"));
    return () => subscription.remove();
  });
}
