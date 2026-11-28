// Packs dist/ (scripts/build.ts) into two zips, the version read from the manifest; the same files give the same bytes:
// - dist/emberhut-extension-<version>.zip, for loading unpacked: its manifest keeps the `key` that fixes the
//   extension's id. The server serves it (EMBERHUT_CHROME_EXTENSION_ZIP), and the Mac helper unpacks it.
// - dist/emberhut-extension-<version>-store.zip, the Chrome Web Store's upload: the same files, the manifest without
//   `key` (release.ts).
// Only what the extension runs goes in: no source maps, sources or dotfiles (release.ts refuses them).

import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { packProblem, storeManifest } from "./release.ts";
import { zip, type ZipEntry } from "./zip-file.ts";

const dist = fileURLToPath(new URL("../dist/", import.meta.url));
const manifestJson = readFileSync(`${dist}manifest.json`, "utf8");
const manifest = JSON.parse(manifestJson) as { version?: unknown };
if (typeof manifest.version !== "string") throw new Error("dist/manifest.json has no version: run the build first");
const base = `emberhut-extension-${manifest.version}`;

const entries: ZipEntry[] = readdirSync(dist, { recursive: true, withFileTypes: true })
  .filter((entry) => entry.isFile() && !entry.name.endsWith(".zip"))
  .map((entry) => {
    const path = `${entry.parentPath}/${entry.name}`.slice(dist.length).replace(/^\/+/, "");
    const problem = packProblem(path);
    if (problem) throw new Error(`dist/: ${problem}`);
    return { name: path, data: readFileSync(`${dist}${path}`) };
  });
const store = entries.map((entry) =>
  entry.name === "manifest.json" ? { name: entry.name, data: Buffer.from(storeManifest(manifestJson)) } : entry,
);

writeFileSync(`${dist}${base}.zip`, zip(entries));
writeFileSync(`${dist}${base}-store.zip`, zip(store));
process.stdout.write(`dist/${base}.zip\ndist/${base}-store.zip\n`);
