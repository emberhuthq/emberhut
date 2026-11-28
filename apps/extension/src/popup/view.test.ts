import { describe, expect, it } from "vitest";
import type { Status } from "../status";
import { builtIn, driving, headline } from "./view";

const base: Status = {
  paired: true,
  name: "work",
  label: "Work",
  server: "https://z.example",
  version: "0.1.0",
  connected: true,
  error: "",
  paused: null,
  driving: false,
  handoff: null,
  notice: null,
  steps: [],
  blocked: [],
  defaults: ["1password.com"],
  tabs: [],
  offer: null,
  notifications: false,
};

describe("the popup's words", () => {
  it("names the profile and the link", () => {
    expect(headline(base)).toBe("Paired as “Work” · connected · v0.1.0");
    expect(headline({ ...base, label: null, connected: false })).toBe("Paired as “work” · not connected · v0.1.0");
    expect(headline({ ...base, paired: false })).toBe("Not paired yet");
  });

  it("says whether Emberhut drives, and what is always blocked", () => {
    expect(driving({ ...base, driving: true })).toBe("Emberhut is driving in this Chrome now.");
    expect(driving(base)).toBe("Emberhut may work in this Chrome whenever it needs to.");
    expect(builtIn(base)).toBe("Always blocked: 1password.com, and chrome:// pages.");
  });
});
