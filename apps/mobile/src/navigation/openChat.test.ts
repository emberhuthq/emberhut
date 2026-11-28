import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { chatTarget } from "./openChat";

// The stack router the app's native stack runs, as @react-navigation/native resolves it.
const app = createRequire(import.meta.url);
const routersPath = createRequire(app.resolve("@react-navigation/native")).resolve("@react-navigation/routers");
// Typed through the native package, which re-exports the routers it runs.
type Router = Pick<typeof import("@react-navigation/native"), "StackActions" | "StackRouter" | "CommonActions">;
const { StackActions, StackRouter, CommonActions } = (await import(pathToFileURL(routersPath).href)) as Router;

const routeNames = ["Tabs", "Rooms", "Task"];
const router = StackRouter({});
const options = { routeNames, routeParamList: {}, routeGetIdList: {} };
const stack = (...names: string[]) =>
  router.getRehydratedState({ routes: names.map((name) => ({ name, key: `${name}-1` })), index: names.length - 1 }, options);

describe("opening a room from the rooms sheet", () => {
  it("closes the sheet and switches the chat under it, with no second tab bar", () => {
    const next = router.getStateForAction(stack("Tabs", "Rooms"), StackActions.popTo(...chatTarget({ room: 7 })), options);
    expect(next?.routes.map((r) => r.name)).toEqual(["Tabs"]);
    expect(next?.routes[0]).toMatchObject({ key: "Tabs-1", params: { screen: "Chat", params: { room: 7 } } });
  });

  it("closes a task too when its Talk opens the chat with a draft", () => {
    const next = router.getStateForAction(stack("Tabs", "Task"), StackActions.popTo(...chatTarget({ room: 3, draft: "About" })), options);
    expect(next?.routes.map((r) => r.name)).toEqual(["Tabs"]);
    expect(next?.routes[0]?.params).toEqual({ screen: "Chat", params: { room: 3, draft: "About" } });
  });

  it("is what navigate got wrong: it pushes another Tabs over the sheet", () => {
    const pushed = router.getStateForAction(stack("Tabs", "Rooms"), CommonActions.navigate(...chatTarget({ room: 7 })), options);
    expect(pushed?.routes.map((r) => r.name)).toEqual(["Tabs", "Rooms", "Tabs"]);
  });
});
