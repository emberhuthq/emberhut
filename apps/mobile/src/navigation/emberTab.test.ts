import { describe, expect, it } from "vitest";
import { EMBER_TITLE, emberTabOptions } from "./emberTab";

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

  it("shows the ember plain until the state says", () => {
    expect(emberTabOptions(undefined, faces).tabBarIcon()).toBe("ember");
  });
});
