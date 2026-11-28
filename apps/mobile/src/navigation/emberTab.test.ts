import { describe, expect, it } from "vitest";
import type { EmberhutState } from "@/api";
import { emberName } from "@/features/ember/name";
import { EMBER_TITLE, emberTabOptions, tintedFaces } from "./emberTab";

const faces = { on: "ember", off: "ember off duty" };

describe("your ember's tab", () => {
  it("is titled Your ember and draws the ember untinted", () => {
    const options = emberTabOptions(true, faces);
    expect(options.title).toBe("Your ember");
    expect(EMBER_TITLE).toBe("Your ember");
    expect(options.tabBarIconRenderingMode).toBe("original");
    expect(options.tabBarIcon()).toBe("ember");
  });

  it("dims the ember while it's off duty", () => {
    expect(emberTabOptions(false, faces).tabBarIcon()).toBe("ember off duty");
  });

  it("is titled with the name you gave it", () => {
    expect(emberTabOptions(true, faces, "Juniper").title).toBe("Juniper");
    expect(emberTabOptions(true, faces, "Juniper the Brave").title).toBe("Juniper the…");
  });

  it("shows the ember plain until the state says", () => {
    expect(emberTabOptions(undefined, faces).tabBarIcon()).toBe("ember");
  });
});

describe("a rename made elsewhere", () => {
  it("shows at the next poll: the title is read from each state the poll brings, nothing kept from before", () => {
    // The state as the poll brings it, nothing else: there is no cache of the name apart from it.
    const polled = (name: EmberhutState["emberName"]): Pick<EmberhutState, "emberName"> => ({ emberName: name });
    const before = polled({ name: "Your ember", custom: false, at: null });
    const after = polled({ name: "Juniper", custom: true, at: "2026-11-28T10:00:00.000Z" });
    expect(emberTabOptions(true, faces, emberName(before)).title).toBe("Your ember");
    expect(emberTabOptions(true, faces, emberName(after)).title).toBe("Juniper");
    expect(emberTabOptions(true, faces, emberName(polled(null))).title).toBe("Your ember");
  });
});

describe("your ember's tab in its tint", () => {
  const tinted = { ember: "ember", coral: "coral ember" };

  it("draws the ember in the tint its look names", () => {
    expect(emberTabOptions(true, tintedFaces("coral", tinted, "off")).tabBarIcon()).toBe("coral ember");
  });

  it("draws a tint this build has no picture of as the ember's own colours, and off duty grey whatever the tint", () => {
    expect(emberTabOptions(true, tintedFaces("calm", tinted, "off")).tabBarIcon()).toBe("ember");
    expect(emberTabOptions(false, tintedFaces("coral", tinted, "off")).tabBarIcon()).toBe("off");
  });
});
