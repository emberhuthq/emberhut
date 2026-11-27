// Packs dist/ (scripts/build.ts) into dist/emberhut-extension-<version>.zip, the version read from the manifest. The same
// files give the same zip. The server serves it (EMBERHUT_CHROME_EXTENSION_ZIP), and the Mac helper unpacks it.

import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { zip, type ZipEntry } from "./zip-file.ts";

const dist = fileURLToPath(new URL("../dist/", import.meta.url));
const manifest = JSON.parse(readFileSync(`${dist}manifest.json`, "utf8")) as { version?: unknown };
if (typeof manifest.version !== "string") throw new Error("dist/manifest.json has no version: run the build first");
const name = `emberhut-extension-${manifest.version}.zip`;

const entries: ZipEntry[] = readdirSync(dist, { recursive: true, withFileTypes: true })
  .filter((entry) => entry.isFile() && !entry.name.endsWith(".zip"))
  .map((entry) => {
    const path = `${entry.parentPath}/${entry.name}`.slice(dist.length).replace(/^\/+/, "");
    return { name: path, data: readFileSync(`${dist}${path}`) };
  });
writeFileSync(`${dist}${name}`, zip(entries));
process.stdout.write(`dist/${name}\n`);
