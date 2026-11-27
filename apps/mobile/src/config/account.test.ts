import { describe, expect, it } from "vitest";
import { checkAccountSettings } from "../../app.config";
import { accountConfig, readAccountSettings } from "./account";

const key = "a-publishable-key";

describe("the account's build-time switch", () => {
  it("is off when neither setting is there, as in builds from source", () => {
    expect(readAccountSettings({})).toEqual({ ok: true, account: null });
    expect(readAccountSettings({ publishableKey: " ", directoryUrl: "" })).toEqual({ ok: true, account: null });
    // The tests run without them, as CI and source builds do.
    expect(accountConfig).toBeNull();
  });

  it("is on with both, the directory as its https origin", () => {
    expect(readAccountSettings({ publishableKey: key, directoryUrl: "https://Relay.Example.com/" })).toEqual({
      ok: true,
      account: { publishableKey: key, directoryUrl: "https://relay.example.com", passkeys: false },
    });
    expect(readAccountSettings({ publishableKey: key, directoryUrl: "https://relay.example.com:8443", passkeys: "1" })).toEqual({
      ok: true,
      account: { publishableKey: key, directoryUrl: "https://relay.example.com:8443", passkeys: true },
    });
  });

  it("refuses half a setup, a directory that isn't an https origin, and passkeys without the account", () => {
    for (const env of [
      { publishableKey: key },
      { directoryUrl: "https://relay.example.com" },
      { publishableKey: key, directoryUrl: "http://relay.example.com" },
      { publishableKey: key, directoryUrl: "https://relay.example.com/v1" },
      { publishableKey: key, directoryUrl: "https://user@relay.example.com" },
      { publishableKey: "two words", directoryUrl: "https://relay.example.com" },
      { publishableKey: key, directoryUrl: "https://relay.example.com", passkeys: "yes" },
      { passkeys: "1" },
    ])
      expect(readAccountSettings(env).ok, JSON.stringify(env)).toBe(false);
  });

  it("stops the build when the settings are wrong", () => {
    expect(() => checkAccountSettings({ EXPO_PUBLIC_EMBERHUT_CLERK_PUBLISHABLE_KEY: key })).toThrow(/or neither/);
    expect(() => checkAccountSettings({})).not.toThrow();
  });
});
