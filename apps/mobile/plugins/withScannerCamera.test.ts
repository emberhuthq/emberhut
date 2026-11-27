import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";

type Feature = { $: Record<string, string> };
type Manifest = { manifest: { $: Record<string, string>; "uses-feature"?: Feature[] } };
type Property = { type: "property"; key: string; value: string } | { type: "comment"; value: string };

// The plugin is CommonJS (Expo loads it with require); so is its test's view of it.
const plugin = createRequire(import.meta.url)("./withScannerCamera.js") as {
  applyManifest: (manifest: Manifest) => Manifest;
  applyProperties: (properties: Property[]) => Property[];
};

const manifest = (): Manifest => ({ manifest: { $: { "xmlns:android": "http://schemas.android.com/apk/res/android" } } });

describe("withScannerCamera", () => {
  it("declares the camera optional, once", () => {
    const once = plugin.applyManifest(manifest());
    expect(once.manifest["uses-feature"]).toEqual([{ $: { "android:name": "android.hardware.camera", "android:required": "false" } }]);
    expect(plugin.applyManifest(once).manifest["uses-feature"]).toHaveLength(1);
    const required = manifest();
    required.manifest["uses-feature"] = [{ $: { "android:name": "android.hardware.camera", "android:required": "true" } }];
    expect(plugin.applyManifest(required).manifest["uses-feature"]).toEqual(once.manifest["uses-feature"]);
  });

  it("turns expo-camera's document scanner off, and leaves other properties alone", () => {
    const properties: Property[] = [{ type: "property", key: "hermesEnabled", value: "true" }];
    const applied = plugin.applyProperties(properties);
    expect(applied).toContainEqual({ type: "property", key: "expo.camera.document-scanner-enabled", value: "false" });
    expect(applied).toContainEqual({ type: "property", key: "hermesEnabled", value: "true" });
    expect(plugin.applyProperties(applied).filter((p) => p.type === "property" && p.key.startsWith("expo.camera"))).toHaveLength(1);
  });
});
