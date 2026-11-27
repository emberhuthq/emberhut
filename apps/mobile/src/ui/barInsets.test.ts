import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";

// useBarInsets reads the header's height from @react-navigation/elements' HeaderHeightContext (provided by each tab's
// native stack) and the tab bar's from react-native-bottom-tabs' BottomTabBarHeightContext (provided by its navigator),
// both matched by identity: the app's copies must be the ones the navigators provide, or the insets silently drop to 0.
const app = createRequire(import.meta.url);
const from = (pkg: string, dep: string) => createRequire(app.resolve(`${pkg}/package.json`)).resolve(dep);

describe("bar insets", () => {
  it("read the same @react-navigation/elements as the tabs' native stacks", () => {
    expect(from("@react-navigation/native-stack", "@react-navigation/elements")).toBe(app.resolve("@react-navigation/elements"));
  });

  it("read the same react-native-bottom-tabs as the tab navigator", () => {
    expect(from("@bottom-tabs/react-navigation", "react-native-bottom-tabs")).toBe(app.resolve("react-native-bottom-tabs"));
  });
});
