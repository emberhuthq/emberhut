import { describe, expect, it, vi } from "vitest";
import { createDialect, parseDialect, type PushAddress } from "@/api";
import { createSessionStore } from "@/state/session";
import { fakeApi, fixedAddress, json, tokens } from "@/test/fakes";
import { linkOf, tappedLink } from "./link";
import { apnsEnv, createPushStore, type PushPermission, type PushPlatform } from "./registration";

/**
 * A phone as push sees it: a permission it grants or refuses when asked (Android lets the app ask again after a first
 * no), and a push token that can turn over or be voided. An Android phone's address is its FCM token; an iPhone's is its
 * APNs token with the service the build registered with.
 */
function standIn(
  answer: boolean | "noFirebase" = true,
  permission: PushPermission = "undetermined",
  address: (token: string) => PushAddress = (token) => ({ token, platform: "android" }),
) {
  let token = "fcm-1";
  let voided = 0;
  let listener: ((token: string) => void) | null = null;
  const platform: PushPlatform = {
    prepare: vi.fn(async () => undefined),
    permission: async () => ({ status: permission, canAskAgain: true }),
    requestPermission: vi.fn(async () => {
      permission = answer === false ? "denied" : "granted";
      return answer !== false;
    }),
    forget: vi.fn(async () => {
      voided += 1;
      token = `fcm-new-${voided}`;
    }),
    token: async () => {
      if (answer === "noFirebase") throw new Error("no google-services.json");
      return token;
    },
    address: async (t) => address(t),
    onTokenChange: (l) => {
      listener = l;
      return () => (listener = null);
    },
  };
  return {
    platform,
    turnOver(next: string) {
      token = next;
      listener?.(next);
    },
  };
}

function setup(
  answer: boolean | "noFirebase" = true,
  devices: () => Response | Promise<Response> = () => json(200, { ok: true }),
  permission: PushPermission = "undetermined",
  address?: (token: string) => PushAddress,
) {
  const { server, auth, api, clock } = fakeApi();
  server.on("POST", "/api/auth/pair", () => json(200, tokens(1, clock.now)));
  server.on("POST", "/api/devices", devices);
  const session = createSessionStore({ auth, api, address: fixedAddress(), device: { name: "Pixel", model: "Pixel" } });
  const phone = standIn(answer, permission, address);
  const push = createPushStore({ platform: phone.platform, register: (t) => api.devices.registerPush(t), session });
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  return { server, session, push, phone, settle, registered: () => server.to("/api/devices").map((r) => r.body) };
}

describe("push registration", () => {
  it("sends nothing while signed out", async () => {
    const { session, push, registered } = setup();
    await session.getState().start();
    await push.getState().sync();
    expect(registered()).toEqual([]);
  });

  it("after sign-in makes Emberhut's channel, asks for the permission and gives Emberhut the FCM token once", async () => {
    const { session, push, phone, settle, registered } = setup();
    await session.getState().start();
    await session.getState().pair("4F7K-92QX");
    await settle();
    await push.getState().sync();
    expect(phone.platform.prepare).toHaveBeenCalled();
    expect(registered()).toEqual([{ token: "fcm-1", platform: "android" }]);
    expect(push.getState()).toMatchObject({ permission: "granted", registered: true, error: null });
  });

  it("gives Emberhut the new token when FCM turns it over", async () => {
    const { session, phone, settle, registered } = setup();
    await session.getState().start();
    await session.getState().pair("4F7K-92QX");
    await settle();
    phone.turnOver("fcm-2");
    await settle();
    expect(registered().map((b) => (b as { token: string }).token)).toEqual(["fcm-1", "fcm-2"]);
  });

  it("sends a token once when FCM hands it over while the sync is sending it", async () => {
    let answer: (response: Response) => void = () => undefined;
    const held = new Promise<Response>((resolve) => (answer = resolve));
    const { session, phone, settle, registered } = setup(true, () => held);
    await session.getState().start();
    await session.getState().pair("4F7K-92QX");
    await settle();
    expect(registered()).toHaveLength(1);
    phone.turnOver("fcm-1");
    answer(json(200, { ok: true }));
    await settle();
    expect(registered()).toHaveLength(1);
  });

  it("on an iPhone gives Emberhut the APNs token and its service, with no platform", async () => {
    const apns = "a".repeat(64);
    const { session, push, phone, settle, registered } = setup(true, undefined, "undetermined", (token) => ({ token, env: "production" }));
    vi.spyOn(phone.platform, "token").mockResolvedValue(apns);
    await session.getState().start();
    await session.getState().pair("4F7K-92QX");
    await settle();
    expect(phone.platform.requestPermission).toHaveBeenCalledOnce();
    expect(registered()).toEqual([{ token: apns, env: "production" }]);
    expect(push.getState()).toMatchObject({ permission: "granted", registered: true, error: null });
  });

  it("on an iPhone gives Emberhut the new APNs token when it changes", async () => {
    const { session, phone, settle, registered } = setup(true, undefined, "undetermined", (token) => ({ token, env: "sandbox" }));
    await session.getState().start();
    await session.getState().pair("4F7K-92QX");
    await settle();
    phone.turnOver("b".repeat(64));
    await settle();
    expect(registered()).toEqual([
      { token: "fcm-1", env: "sandbox" },
      { token: "b".repeat(64), env: "sandbox" },
    ]);
  });

  it("sends nothing when you say no, and asks only once a launch", async () => {
    const { session, push, phone, settle, registered } = setup(false);
    await session.getState().start();
    await session.getState().pair("4F7K-92QX");
    await settle();
    await push.getState().sync();
    expect(registered()).toEqual([]);
    expect(push.getState().permission).toBe("denied");
    expect(phone.platform.requestPermission).toHaveBeenCalledOnce();
  });

  it("doesn't ask again on a later launch once you said no, though Android would still show the prompt", async () => {
    const { session, push, phone, settle, registered } = setup(true, undefined, "denied");
    await session.getState().start();
    await session.getState().pair("4F7K-92QX");
    await settle();
    expect(phone.platform.requestPermission).not.toHaveBeenCalled();
    expect(push.getState().permission).toBe("denied");
    expect(registered()).toEqual([]);
  });

  it("says so when the build has no push, and tries again after Emberhut couldn't be reached", async () => {
    const none = setup("noFirebase");
    await none.session.getState().start();
    await none.session.getState().pair("4F7K-92QX");
    await none.settle();
    expect(none.push.getState()).toMatchObject({ registered: false, error: "This build has no push set up" });

    let up = false;
    const flaky = setup(true, () => (up ? json(200, { ok: true }) : json(500, { error: "busy" })));
    await flaky.session.getState().start();
    await flaky.session.getState().pair("4F7K-92QX");
    await flaky.settle();
    expect(flaky.push.getState()).toMatchObject({ registered: false, error: "busy" });
    up = true;
    await flaky.push.getState().sync();
    expect(flaky.push.getState().registered).toBe(true);
  });

  it("says so when the channel can't be made, and tries again on the next sync", async () => {
    const { session, push, phone, settle, registered } = setup();
    vi.mocked(phone.platform.prepare).mockRejectedValueOnce(new Error("no notification channels"));
    await session.getState().start();
    await session.getState().pair("4F7K-92QX"); // its sync is the one that fails
    await settle();
    expect(push.getState()).toMatchObject({ registered: false, error: "Push couldn't start: no notification channels" });
    expect(registered()).toEqual([]);
    await expect(push.getState().sync()).resolves.toBeUndefined();
    expect(push.getState()).toMatchObject({ permission: "granted", registered: true, error: null });
  });

  it("says so when asking for the permission fails, and the sync still resolves", async () => {
    const { session, push, phone, settle, registered } = setup();
    vi.mocked(phone.platform.requestPermission).mockRejectedValueOnce("denied by the system");
    await session.getState().start();
    await session.getState().pair("4F7K-92QX");
    await settle();
    await expect(push.getState().sync()).resolves.toBeUndefined();
    expect(push.getState().error).toBe("Push couldn't start: the phone said no");
    expect(registered()).toEqual([]);
  });

  it("voids the token on sign-out, so the next sign-in registers a new one", async () => {
    const { session, push, phone, settle, registered } = setup();
    await session.getState().start();
    await session.getState().pair("4F7K-92QX");
    await settle();
    await session.getState().signOut();
    expect(phone.platform.forget).toHaveBeenCalledOnce();
    expect(push.getState().registered).toBe(false);
    await session.getState().pair("4F7K-92QX");
    await settle();
    expect(registered().map((b) => (b as { token: string }).token)).toEqual(["fcm-1", "fcm-new-1"]);
  });
});

describe("an iPhone's APNs service", () => {
  it("is the sandbox for a development build's profile and in the Simulator, production otherwise", () => {
    expect(apnsEnv("development", true)).toBe("sandbox");
    // An ad hoc profile.
    expect(apnsEnv("production", true)).toBe("production");
    // TestFlight and the App Store: no embedded profile.
    expect(apnsEnv(null, true)).toBe("production");
    expect(apnsEnv(undefined, true)).toBe("production");
    expect(apnsEnv(null, false)).toBe("sandbox");
  });
});

describe("a tapped notification", () => {
  it("opens the app's own links and nothing else", () => {
    expect(linkOf({ link: "emberhut://chat?room=4", room: "4" })).toBe("emberhut://chat?room=4");
    expect(linkOf({ link: "other://chat?room=4" })).toBeNull();
    expect(linkOf({ link: "https://example.com" })).toBeNull();
    expect(linkOf({})).toBeNull();
    expect(linkOf(undefined)).toBeNull();
  });

  it("finds the link where each platform puts the server's data", () => {
    // Android, the app running: the content's data.
    expect(tappedLink({ link: "emberhut://chat?room=4" }, { type: "push" })).toBe("emberhut://chat?room=4");
    // Android, shown by the system with the app closed: only in FCM's message.
    expect(tappedLink({}, { type: "push", remoteMessage: { data: { link: "emberhut://task/12" } } })).toBe("emberhut://task/12");
    // iPhone: the content's data is only an Expo push's body; the server's keys sit beside aps in the payload.
    const apns = { aps: { alert: { title: "Emberhut", body: "Done" } }, link: "emberhut://chat?room=4", room: 4 };
    expect(tappedLink(null, { type: "push", payload: apns })).toBe("emberhut://chat?room=4");
    expect(tappedLink(null, { type: "push", payload: { aps: {} } })).toBeNull();
    expect(tappedLink(null, null)).toBeNull();
  });

  it("reads a link in a server's dialect as the app's own", () => {
    const words = createDialect(parseDialect(JSON.stringify({ prefixes: [["other://", "emberhut://"]] })));
    expect(linkOf({ link: "other://task/12" }, words)).toBe("emberhut://task/12");
    expect(linkOf({ link: "emberhut://task/12" }, words)).toBe("emberhut://task/12");
    expect(linkOf({ link: "https://example.com" }, words)).toBeNull();
    expect(tappedLink(null, { type: "push", payload: { link: "other://task/12" } }, words)).toBe("emberhut://task/12");
  });
});
