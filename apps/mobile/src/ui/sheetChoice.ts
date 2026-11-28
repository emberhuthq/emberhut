/**
 * When a sheet's tapped action runs. On iOS a sheet is a modal, and nothing can be presented over a modal that is still
 * going away: a picker or share sheet opened by the tap goes with it, shown nowhere, and its promise never settles. So
 * there the action waits until the sheet is gone (the Modal's onDismiss). Android's Modal has no onDismiss and opens a
 * picker over a closing sheet just fine: the action runs at once.
 */
export function sheetChoice(waitForDismiss: boolean) {
  let chosen: (() => void) | null = null;
  return {
    choose(action: () => void) {
      if (waitForDismiss) chosen = action;
      else action();
    },
    dismissed() {
      const action = chosen;
      chosen = null;
      action?.();
    },
  };
}
