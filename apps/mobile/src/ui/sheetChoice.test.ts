import { describe, expect, it, vi } from "vitest";
import { sheetChoice } from "./sheetChoice";

describe("a sheet's tapped action", () => {
  it("on iOS opens the photo picker only once the sheet is gone, and once", () => {
    const openPicker = vi.fn();
    const sheet = sheetChoice(true);
    sheet.choose(openPicker);
    expect(openPicker).not.toHaveBeenCalled();
    sheet.dismissed();
    expect(openPicker).toHaveBeenCalledTimes(1);
    sheet.dismissed();
    expect(openPicker).toHaveBeenCalledTimes(1);
  });

  it("on iOS runs nothing when the sheet is closed without a choice", () => {
    const openPicker = vi.fn();
    const sheet = sheetChoice(true);
    sheet.dismissed();
    sheet.choose(openPicker);
    expect(openPicker).not.toHaveBeenCalled();
  });

  it("on iOS runs the last action tapped before the sheet went", () => {
    const photos = vi.fn();
    const files = vi.fn();
    const sheet = sheetChoice(true);
    sheet.choose(photos);
    sheet.choose(files);
    sheet.dismissed();
    expect(photos).not.toHaveBeenCalled();
    expect(files).toHaveBeenCalledTimes(1);
  });

  it("on iOS runs the action when a sheet taken off the screen never says it's gone", () => {
    const answer = vi.fn();
    const sheet = sheetChoice(true);
    sheet.choose(answer);
    // A caller that unmounts its sheet on close: the Modal's onDismiss never comes, its unmount does.
    sheet.unmounted();
    expect(answer).toHaveBeenCalledTimes(1);
    sheet.dismissed();
    expect(answer).toHaveBeenCalledTimes(1);
  });

  it("on Android runs at once: its Modal never says it's gone", () => {
    const openPicker = vi.fn();
    sheetChoice(false).choose(openPicker);
    expect(openPicker).toHaveBeenCalledTimes(1);
  });
});
