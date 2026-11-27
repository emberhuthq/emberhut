import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";

// The plugin is CommonJS (Expo loads it with require); so is its test's view of it.
const plugin = createRequire(import.meta.url)("./withReleaseSigning.js") as {
  apply: (gradle: string) => string;
  envLine: string;
  signingProperties: string[];
};

// android/app/build.gradle as Expo SDK 58's prebuild template has it (expo's template.tgz), unchanged.
const template = readFileSync(new URL("./fixtures/app-build.gradle", import.meta.url), "utf8");
const count = (text: string, part: string) => text.split(part).length - 1;

describe("withReleaseSigning", () => {
  it("adds the loader once, before android { }, and is idempotent", () => {
    const once = plugin.apply(template);
    expect(count(once, "// Emberhut release signing")).toBe(1);
    expect(once.indexOf("// Emberhut release signing")).toBeLessThan(once.search(/^android\s*\{/m));
    expect(plugin.apply(once)).toBe(once);
  });

  it("signs release builds with the release config when the key is set, and debug builds as before", () => {
    const signed = plugin.apply(template);
    const buildTypes = signed.slice(signed.indexOf("buildTypes {"));
    const release = buildTypes.slice(buildTypes.indexOf("release {"));
    expect(release).toMatch(/^release \{[\s\S]*?signingConfig emberhutSigning != null \? signingConfigs\.release : signingConfigs\.debug/);
    expect(buildTypes.slice(0, buildTypes.indexOf("release {"))).toContain("signingConfig signingConfigs.debug");
    const configs = signed.slice(signed.indexOf("signingConfigs {"), signed.indexOf("buildTypes {"));
    for (const name of plugin.signingProperties) expect(configs).toContain(`emberhutSigning.getProperty("${name}")`);
    // Every property is checked by name before the keystore is opened, so a missing one is a clear error.
    expect(signed).toContain('for (name in ["storeFile", "storePassword", "keyAlias", "keyPassword"])');
    expect(signed).toContain('throw new GradleException(emberhutSigningPath + " has no " + name)');
    expect(signed).toContain('graph.allTasks.any { it.name == "bundleRelease" }');
  });

  it("refuses a template it no longer knows, instead of leaving release debug-signed", () => {
    expect(() => plugin.apply(template.replace(/signingConfigs\s*\{/, "keys {"))).toThrow(/no longer has the template's signingConfigs/);
    expect(() => plugin.apply(template.replace(/signingConfig signingConfigs\.debug/g, ""))).toThrow(/release buildType/);
    expect(() => plugin.apply("")).toThrow();
  });

  it("reads the .env path whole, with spaces, without its quotes", () => {
    const pathOf = (line: string) => new RegExp(plugin.envLine).exec(line)?.[2];
    expect(pathOf("EMBERHUT_ANDROID_KEYSTORE_PROPERTIES=/keys/emberhut.properties")).toBe("/keys/emberhut.properties");
    expect(pathOf('  EMBERHUT_ANDROID_KEYSTORE_PROPERTIES = "/My Keys/emberhut.properties"  ')).toBe("/My Keys/emberhut.properties");
    expect(pathOf("EMBERHUT_ANDROID_KEYSTORE_PROPERTIES='/keys/a b.properties'")).toBe("/keys/a b.properties");
    expect(pathOf("EMBERHUT_ANDROID_KEYSTORE_PROPERTIES=")).toBeUndefined();
    expect(pathOf("# EMBERHUT_ANDROID_KEYSTORE_PROPERTIES=/x")).toBeUndefined();
    expect(pathOf("OTHER=/x")).toBeUndefined();
  });
});
