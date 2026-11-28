// What the runner adds to a session's system prompt and says to it on its own: who started it, how to ask the owner
// something, where files sent to it land, and the short notes after an account move or a restart. Each installation
// may replace the first two and add a note about its machine (config.json's `instructions`).
import type { Instructions } from "../config.ts";

/** For a session the assistant opens for work, unless the configuration says otherwise. */
export const WORK =
  "This session was started by the Emberhut assistant on this runner. Its owner can read it in the Emberhut app and reply here. " +
  "Work as you would in the owner's own Claude Code session, with full access.";

/** For the owner's own session, unless the configuration says otherwise. */
export const DIRECT =
  "This is the owner's own Claude Code session, opened and driven from the Emberhut app, like a terminal session. Work with them as " +
  "you would there, with full access. The Emberhut assistant can read this session and sometimes writes in it for the owner: such " +
  'messages start with "(From the assistant"; treat them as passed on from the owner. To ask something with choices, use ' +
  "AskUserQuestion: it shows in the app and the session waits for the answer.";

/** What an owner's session is told after it asks a question: the card is out, the answer comes as the next message. */
export const ASKED =
  "Your question is in front of the owner now, in the Emberhut app, with its options. End your turn here with a short line; the " +
  "answer comes as the next message.";

/** Carry on after the session moved to another account mid-turn. */
export const MOVED =
  "(The runner: the account this session ran on hit its usage limit, so the session moved to another account. Nothing is lost. " +
  "Carry on exactly where you stopped, with the same task, and end your reply as asked before.)";

/** Carry on after the runner restarted mid-turn. */
export const RESTARTED =
  "(The runner restarted while you were working on this turn, so the command or step you were in was cut off before it finished. " +
  "Check where things stand, run again what didn't finish, and carry on with the same task; end your reply as asked before.)";

/** Carry on after a restart stopped the background jobs the session waited on. */
export const JOBS_LOST =
  "(The runner restarted, which stopped the background jobs this session was waiting on. Check what they were doing, run again " +
  "what didn't finish, and carry on with the same task; end your reply as asked before.)";

/** Carry on once an account is free again after every one was out. */
export const FREE_AGAIN = "(The runner: an account is free again. Carry on where you stopped, with the same task.)";

/** Where files sent to the session land. */
export const filesNote = (inbox: string): string =>
  `Files sent to you (designs, spreadsheets, screenshots) land in ${inbox}; you're told when one arrives.`;

/** A session in a worktree outside its project's folder is told to read the project's instructions there. */
export const projectNote = (project: string, files: string[]): string =>
  `This session works in a worktree of ${project}. Before anything else, read the project's instructions there (${files.join(", ")}): ` +
  "they apply here as if you had started in that folder.";

/** The text appended to the agent's own system prompt for a session. */
export function systemAppend(ins: Instructions, direct: boolean, inbox: string, project: string | null): string {
  return ["", direct ? (ins.direct ?? DIRECT) : (ins.work ?? WORK), ins.machine, filesNote(inbox), project]
    .filter((x): x is string => x !== undefined && x !== null)
    .join("\n\n");
}
