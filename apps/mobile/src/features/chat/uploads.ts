import type { ChatFile } from "@/api";
import { extensionOf, viewerKind } from "@/features/files/files";

// What you attach, from the picker to the server: its name (the picker's, or one made up from its type), its kind, and
// where each upload stands.

export interface Picked {
  uri: string;
  name?: string | null;
  mimeType?: string | null;
}

const fromMime: Record<string, string> = {
  "image/jpeg": "jpg",
  "image/png": "png",
  "image/heic": "heic",
  "image/webp": "webp",
  "image/gif": "gif",
  "video/mp4": "mp4",
  "video/quicktime": "mov",
  "application/pdf": "pdf",
};

export function uploadKind(p: Picked): ChatFile["kind"] {
  const mime = p.mimeType ?? "";
  if (mime.startsWith("image/")) return "image";
  if (mime.startsWith("video/")) return "video";
  const byName = viewerKind(p.name ?? p.uri);
  return byName === "image" || byName === "video" ? byName : "file";
}

/** The picker's name; else "photo-<seconds>.jpg" and the like, from its type or its address. */
export function uploadName(p: Picked, now: number): string {
  const given = p.name?.trim();
  if (given) return given.replace(/\//g, "-");
  const kind = uploadKind(p);
  const ext =
    fromMime[p.mimeType ?? ""] ??
    (extensionOf(p.uri.split(/[?#]/)[0] ?? "") || (kind === "image" ? "jpg" : kind === "video" ? "mp4" : "bin"));
  return `${kind === "image" ? "photo" : kind === "video" ? "video" : "file"}-${Math.floor(now / 1000)}.${ext}`;
}

/** Emberhut takes files up to this size; a bigger one is refused on the phone, before it starts going up. */
export const maxUploadBytes = 500 * 1024 * 1024;

/** Why a file can't go up, or null when it can. */
export function uploadRefusal(size: number): string | null {
  return size > maxUploadBytes ? "Over 500 MB: too big to send" : null;
}

/** Why an upload failed, in the server's words when it gave some (too large, not stored, signed out). */
export const uploadFailure = (cause: unknown): string | undefined => (cause instanceof Error && cause.message ? cause.message : undefined);

/** A picker that didn't open or failed while picking: the line shown above the message box. */
export function pickFailure(what: "photos" | "files", cause: unknown): string {
  const why = uploadFailure(cause);
  return `Couldn't open the ${what}${why ? `: ${why}` : "."}`;
}

export interface Upload {
  id: string;
  name: string;
  kind: ChatFile["kind"];
  /** The phone's copy: its thumbnail while it goes up. */
  uri: string;
  status: "sending" | "sent" | "failed";
  file?: ChatFile;
  /** Why it failed, when it's known. */
  error?: string;
}

export type UploadAction =
  | { type: "add"; uploads: Upload[] }
  | { type: "sent"; id: string; file: ChatFile }
  | { type: "failed"; id: string; error?: string }
  | { type: "remove"; id: string }
  | { type: "clear" };

export function uploadsReducer(state: Upload[], action: UploadAction): Upload[] {
  switch (action.type) {
    case "add":
      return [...state, ...action.uploads];
    case "sent":
      return state.map((u) => (u.id === action.id ? { ...u, status: "sent", file: action.file } : u));
    case "failed":
      return state.map((u) => (u.id === action.id ? { ...u, status: "failed", error: action.error } : u));
    case "remove":
      return state.filter((u) => u.id !== action.id);
    case "clear":
      return [];
  }
}

/** What goes with the message: the files the server has. Nothing while one is still going up. */
export function readyFiles(uploads: readonly Upload[]): { files: ChatFile[]; sending: boolean } {
  return {
    files: uploads.flatMap((u) => (u.status === "sent" && u.file ? [u.file] : [])),
    sending: uploads.some((u) => u.status === "sending"),
  };
}
