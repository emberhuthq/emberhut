import { RecordingPresets, requestRecordingPermissionsAsync, setAudioModeAsync, useAudioRecorder } from "expo-audio";
import { File } from "expo-file-system";
import { useCallback, useRef, useState } from "react";
import { inBackground } from "@/log";
import { api } from "@/services";

/** The words dictation is told, so the app's own names come out right. */
const words = ["Emberhut", "Claude", "Claude Code", "MCP", "PR", "PRs"];

// Speech, not a tap: a recording smaller than this has nothing to transcribe.
const minBytes = 2_000;

const options = { ...RecordingPresets.HIGH_QUALITY, sampleRate: 16_000, numberOfChannels: 1, bitRate: 64_000 };

export interface Dictation {
  listening: boolean;
  /** The recording is being turned into text. */
  preparing: boolean;
  error: string | null;
  toggle: () => void;
  /** The text was sent or cleared: words still on their way are dropped. */
  discard: () => void;
  clearError: () => void;
}

/**
 * Dictation as the reference client does it: tap, speak (English, Arabic or both), tap again; the recording goes to Emberhut's server
 * for speech to text and the words come back into the message box, after what is already typed.
 */
export function useDictation(onText: (said: string) => void): Dictation {
  const recorder = useAudioRecorder(options);
  const [listening, setListening] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const session = useRef(0);

  const start = useCallback(async () => {
    setError(null);
    try {
      // The permission API itself can fail (a broken audio session); that is a failed start too, never a rejection.
      const permission = await requestRecordingPermissionsAsync();
      if (!permission.granted) {
        setError("Allow the microphone in the phone's settings for this app to dictate.");
        return;
      }
      await setAudioModeAsync({ allowsRecording: true, playsInSilentMode: true });
      await recorder.prepareToRecordAsync();
      recorder.record();
      setListening(true);
    } catch {
      setError("Dictation couldn't start.");
    }
  }, [recorder]);

  const stop = useCallback(async () => {
    setListening(false);
    const id = ++session.current;
    try {
      await recorder.stop();
      await setAudioModeAsync({ allowsRecording: false });
      if (!recorder.uri) return;
      if (new File(recorder.uri).size < minBytes) return;
      setPreparing(true);
      const said = (await api.files.transcribe(recorder.uri, "dictation.m4a", words)).trim();
      if (id === session.current && said !== "") onText(said);
    } catch (cause) {
      if (id === session.current) setError(`Dictation failed: ${cause instanceof Error ? cause.message : "no text"}`);
    } finally {
      if (id === session.current) setPreparing(false);
    }
  }, [recorder, onText]);

  const toggle = useCallback(() => {
    if (listening) inBackground(stop(), "useDictation: stop");
    else if (!preparing) inBackground(start(), "useDictation: start");
  }, [listening, preparing, start, stop]);

  const discard = useCallback(() => {
    session.current += 1;
    setPreparing(false);
    if (listening) {
      setListening(false);
      recorder.stop().catch(() => undefined);
    }
  }, [listening, recorder]);

  return { listening, preparing, error, toggle, discard, clearError: useCallback(() => setError(null), []) };
}
