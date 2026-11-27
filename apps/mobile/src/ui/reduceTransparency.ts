import { createStore } from "zustand/vanilla";

// Reduce Transparency (Settings, Accessibility, Display), read once for the whole app: every glass surface and bar
// reads the one value, which is known before the app's screens first draw, so none draws blur and then turns solid.

/** Where the setting comes from: AccessibilityInfo in the app, a stand-in in the tests. */
export interface TransparencySource {
  read(): Promise<boolean>;
  listen(onChange: (reduce: boolean) => void): () => void;
}

export interface TransparencyState {
  /** Nothing may be see-through. */
  reduce: boolean;
  /** The first read answered (or failed: then as if off). */
  known: boolean;
}

/** How long the screens wait for the first read: past it they draw as if the setting were off. */
export const transparencyWaitMs = 500;

/**
 * One read and one listener for the app's life; a change while the read is out wins over its answer. A read that hasn't
 * answered within `waitMs` counts as off, so a native call that never settles can't keep the app blank; a later answer
 * or change still sets the value.
 */
export function createReduceTransparencyStore(source: TransparencySource, waitMs = transparencyWaitMs) {
  const store = createStore<TransparencyState>()(() => ({ reduce: false, known: false }));
  let changed = false;
  const fallback = setTimeout(() => store.setState({ known: true }), waitMs);
  const settle = (state: Partial<TransparencyState>) => {
    clearTimeout(fallback);
    store.setState({ ...state, known: true });
  };
  source.listen((reduce) => {
    changed = true;
    settle({ reduce });
  });
  source
    .read()
    .then((reduce) => {
      if (!changed) settle({ reduce });
    })
    .catch(() => settle({}));
  return store;
}
