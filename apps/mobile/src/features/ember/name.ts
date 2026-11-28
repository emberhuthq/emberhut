/**
 * What you call your ember ("Pip", "Juniper"): one name on every device, kept by the server per person (`POST
 * /api/ember/name`) and carried in the state as `emberName`. Without one it is "Your ember". The rules here are the
 * server's (`emberhut_api::ember_name::clean_name`), so a name it would refuse is refused before it is sent.
 */

import { ApiError, type EmberName, type EmberNameReply, type EmberhutState } from "@/api";

/** What the ember is called when you gave it no name. */
export const DEFAULT_EMBER_NAME = "Your ember";

/** The most characters a name may have. */
export const EMBER_NAME_MAX = 24;

/** The default, never changed. */
export const defaultEmberName: EmberName = { name: DEFAULT_EMBER_NAME, custom: false, at: null };

/** Your own name for the ember in a state; undefined while it is "Your ember" (or the server is older). */
export function customEmberName(state: Pick<EmberhutState, "emberName"> | undefined): string | undefined {
  const ember = state?.emberName;
  return ember?.custom ? ember.name : undefined;
}

/** What the ember is called in a state: your name for it, or "Your ember". */
export function emberName(state: Pick<EmberhutState, "emberName"> | undefined): string {
  return customEmberName(state) ?? DEFAULT_EMBER_NAME;
}

/** A name as the server keeps it (null: the default), or why it wouldn't take it. */
export type CleanName = { ok: true; name: string | null } | { ok: false; error: string };

// Control characters, and the ones that draw nothing or turn text around (zero-width, direction marks, the BOM).
const notPlain = /[\u0000-\u001f\u007f-\u009f​-‏‪-‮⁠-⁩﻿]/u;

/** `text` trimmed; null when blank or the default's own words (in any case); refused when not one line of 1 to 24. */
export function cleanEmberName(text: string): CleanName {
  const name = text.trim();
  if (name === "" || name.toLowerCase() === DEFAULT_EMBER_NAME.toLowerCase()) return { ok: true, name: null };
  if (notPlain.test(name)) return { ok: false, error: "One line of plain text" };
  if ([...name].length > EMBER_NAME_MAX) return { ok: false, error: `${EMBER_NAME_MAX} characters at most` };
  return { ok: true, name };
}

/** The server's reason without the field's name it starts with (`name: 24 characters at most`). */
function reasonOf(cause: unknown): string {
  if (cause instanceof ApiError && cause.unreachable) return "Can't reach your ember";
  const message = cause instanceof Error ? cause.message : "no reason given";
  const words = message.replace(/^name:\s*/u, "");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** What the rename flow reaches: the name now on screen and the server, its own state, the request. */
export interface RenameIo {
  /** The name on screen now. */
  current: () => EmberName | undefined;
  /** Puts `name` everywhere the ember is named. */
  show: (name: EmberName) => void;
  /** Sends the rename. */
  send: (name: string | null) => Promise<EmberNameReply>;
}

/** How a rename went: the name kept, or why not (and the name went back). */
export type RenameOutcome = { ok: true; name: EmberName } | { ok: false; error: string };

/**
 * Renames the ember to `typed` (blank: back to "Your ember"): shown at once, then sent; a name the server refuses (or
 * one that can't reach it) goes back to the one before, with the reason. A name the rules refuse is never sent.
 */
export async function renameEmber(typed: string, io: RenameIo): Promise<RenameOutcome> {
  const clean = cleanEmberName(typed);
  if (!clean.ok) return { ok: false, error: clean.error };
  const before = io.current() ?? defaultEmberName;
  io.show(clean.name === null ? { ...defaultEmberName, at: before.at } : { name: clean.name, custom: true, at: before.at });
  try {
    const { ok: _ok, error: _error, ...kept } = await io.send(clean.name);
    io.show(kept);
    return { ok: true, name: kept };
  } catch (cause) {
    io.show(before);
    return { ok: false, error: reasonOf(cause) };
  }
}

/** The chat's title: an agent's name in its room, your name for the ember (or the room's own) in Main, else the room's. */
export function chatTitle(room: { main: boolean; name?: string }, agent: string | undefined, custom: string | undefined): string {
  if (agent !== undefined) return agent;
  if (room.main && custom !== undefined) return custom;
  return room.name ?? "Emberhut";
}
