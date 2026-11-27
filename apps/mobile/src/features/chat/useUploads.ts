import * as DocumentPicker from "expo-document-picker";
import { File } from "expo-file-system";
import * as ImagePicker from "expo-image-picker";
import { useCallback, useReducer, useRef, useState } from "react";
import { api } from "@/services";
import {
  pickFailure,
  readyFiles,
  uploadFailure,
  uploadKind,
  uploadName,
  uploadRefusal,
  uploadsReducer,
  type Picked,
  type Upload,
} from "./uploads";

/** Why the file on the device can't go up, or null when it can: too big, or not readable. */
export function refusalOf(uri: string): string | null {
  try {
    return uploadRefusal(new File(uri).size);
  } catch {
    return "Can't read this file";
  }
}

/** What you attach to the message: each file goes up as soon as it's picked, so Send only waits for the last one. */
export function useUploads() {
  const [uploads, dispatch] = useReducer(uploadsReducer, []);
  const [error, setError] = useState<string | null>(null);
  const next = useRef(0);

  const add = useCallback((picked: Picked[]) => {
    const now = Date.now();
    const added: Upload[] = picked.map((p) => ({
      id: `u${++next.current}`,
      name: uploadName(p, now),
      kind: uploadKind(p),
      uri: p.uri,
      status: "sending",
    }));
    dispatch({ type: "add", uploads: added });
    for (const u of added) {
      const refused = refusalOf(u.uri);
      if (refused) {
        dispatch({ type: "failed", id: u.id, error: refused });
        continue;
      }
      // The file itself is the body, sent from disk as it's read (api.files.upload), never whole in memory.
      api.files
        .upload(u.uri, u.name)
        .then((stored) => dispatch({ type: "sent", id: u.id, file: stored }))
        .catch((cause: unknown) => dispatch({ type: "failed", id: u.id, error: uploadFailure(cause) }));
    }
  }, []);

  // A picker that fails (no access to the library, a file it can't copy) says so above the box; it never rejects.
  const pickMedia = useCallback(async () => {
    setError(null);
    try {
      const result = await ImagePicker.launchImageLibraryAsync({
        mediaTypes: ["images", "videos"],
        allowsMultipleSelection: true,
        quality: 1,
      });
      if (!result.canceled) add(result.assets.map((a) => ({ uri: a.uri, name: a.fileName, mimeType: a.mimeType })));
    } catch (cause) {
      setError(pickFailure("photos", cause));
    }
  }, [add]);

  const pickFile = useCallback(async () => {
    setError(null);
    try {
      const result = await DocumentPicker.getDocumentAsync({ multiple: true, copyToCacheDirectory: true });
      if (!result.canceled) add(result.assets.map((a) => ({ uri: a.uri, name: a.name, mimeType: a.mimeType })));
    } catch (cause) {
      setError(pickFailure("files", cause));
    }
  }, [add]);

  return {
    uploads,
    ...readyFiles(uploads),
    pickMedia,
    pickFile,
    /** Why the last pick failed, or null. */
    error,
    clearError: useCallback(() => setError(null), []),
    remove: useCallback((id: string) => dispatch({ type: "remove", id }), []),
  };
}
