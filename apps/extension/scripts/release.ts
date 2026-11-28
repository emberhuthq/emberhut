// The release's rules, pure so the tests can hold them: the version the extension ships with, what may go in its zips,
// and the Chrome Web Store's copy of the manifest.

/** The `version` in the `[workspace.package]` table of the workspace's Cargo.toml, or null. */
export function workspaceVersion(cargoToml: string): string | null {
  const table = /^\[workspace\.package\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(cargoToml)?.[1] ?? "";
  return /^version\s*=\s*"([^"]+)"/m.exec(table)?.[1] ?? null;
}

/** Why the manifest's, package.json's and the workspace's versions don't agree, or null when they do. */
export function versionProblem(manifestJson: string, packageJson: string, cargoToml: string): string | null {
  const manifest = (JSON.parse(manifestJson) as { version?: unknown }).version;
  const pkg = (JSON.parse(packageJson) as { version?: unknown }).version;
  const workspace = workspaceVersion(cargoToml);
  if (workspace === null) return "Cargo.toml has no [workspace.package] version";
  if (manifest !== workspace || pkg !== workspace) {
    return `the extension's version must be the apps' ${workspace}: manifest.json says ${String(manifest)}, package.json ${String(pkg)}`;
  }
  return null;
}

/** Why a file can't go in a release zip (a source map, source, a dotfile, a zip), or null when it can. */
export function packProblem(path: string): string | null {
  const name = path.split("/").pop() ?? path;
  if (name.startsWith(".") || path.split("/").some((part) => part.startsWith("."))) return `${path} is a dotfile`;
  if (/\.(map|ts|tsx|zip|crx|pem)$/i.test(name)) return `${path} doesn't belong in a release`;
  if (!/\.(js|json|html|png)$/i.test(name)) return `${path} isn't a kind of file the extension ships`;
  return null;
}

/**
 * The manifest as the Chrome Web Store takes it: without `key`, which the store refuses (it keeps the item's key
 * itself), the rest as it is, in the same order.
 */
export function storeManifest(manifestJson: string): string {
  const manifest = JSON.parse(manifestJson) as Record<string, unknown>;
  delete manifest.key;
  return `${JSON.stringify(manifest, null, 2)}\n`;
}
