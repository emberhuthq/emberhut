import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { packProblem, storeManifest, versionProblem, workspaceVersion } from "./release";

const here = new URL("..", import.meta.url);
const read = (path: string): string => readFileSync(new URL(path, here), "utf8");

describe("the release", () => {
  it("ships at the apps' version: the manifest, package.json and the workspace agree", () => {
    expect(versionProblem(read("manifest.json"), read("package.json"), read("../../Cargo.toml"))).toBeNull();
  });

  it("reads the workspace's version and says when one disagrees", () => {
    const cargo =
      '[workspace]\nmembers = []\n\n[workspace.package]\nedition = "2024"\nversion = "1.2.3"\n\n[workspace.dependencies]\nversion = "9"\n';
    expect(workspaceVersion(cargo)).toBe("1.2.3");
    expect(workspaceVersion('[package]\nversion = "1"\n')).toBeNull();
    expect(versionProblem('{"version":"1.2.3"}', '{"version":"1.2.3"}', cargo)).toBeNull();
    expect(versionProblem('{"version":"1.2.4"}', '{"version":"1.2.3"}', cargo)).toMatch(/1\.2\.3.*1\.2\.4/);
  });

  it("packs only what the extension runs", () => {
    for (const ok of ["background.js", "popup.html", "manifest.json", "icons/emberhut-16.png"]) expect(packProblem(ok)).toBeNull();
    for (const no of ["background.js.map", "src/main.ts", ".DS_Store", "icons/.keep", "old.zip", "key.pem", "notes.md"])
      expect(packProblem(no)).not.toBeNull();
  });

  it("gives the store the manifest without its key, the rest unchanged", () => {
    const manifest = JSON.parse(read("manifest.json")) as Record<string, unknown>;
    expect(typeof manifest.key).toBe("string");
    const store = JSON.parse(storeManifest(read("manifest.json"))) as Record<string, unknown>;
    expect(store.key).toBeUndefined();
    delete manifest.key;
    expect(store).toEqual(manifest);
    expect(Object.keys(store)).toEqual(Object.keys(manifest));
  });

  it("asks for the fewest permissions: notifications only when the owner turns them on, no host access", () => {
    const manifest = JSON.parse(read("manifest.json")) as Record<string, unknown>;
    expect(manifest.permissions).toEqual(["debugger", "tabs", "tabGroups", "webNavigation", "storage", "alarms"]);
    expect(manifest.optional_permissions).toEqual(["notifications"]);
    expect(manifest.host_permissions).toBeUndefined();
    expect(manifest.content_scripts).toBeUndefined();
    expect((manifest.content_security_policy as { extension_pages: string }).extension_pages).toMatch(
      /^script-src 'self'; object-src 'none'/,
    );
  });
});
