import * as Clipboard from "expo-clipboard";
import type { Decision } from "@/api";
import { draftToCopy } from "@/features/needsYou/model";
import { inBackground } from "@/log";

/** "Send it" puts the exact message on the clipboard, ready to paste where it goes: Emberhut never sends it itself. */
export function copyDraftFor(choice: string, decision: Decision | null): void {
  const draft = draftToCopy(choice, decision);
  if (draft) inBackground(Clipboard.setStringAsync(draft), "copyDraft: setStringAsync");
}
