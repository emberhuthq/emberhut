// Builds the extension into dist/: the service worker and the popup bundled by esbuild, with the manifest, the popup's
// page and the icons beside them. dist/ is the folder Chrome loads unpacked, and what scripts/zip.ts packs. Paths in
// the bundle are relative to this package, whatever the working directory, so the same sources give the same bytes;
// the build fails if the package's own folder shows up in a bundle.
//
// EMBERHUT_WIRE_DIALECT, when set, names a wire dialect file (src/dialect.ts, README.md): it is checked here, a malformed
// one failing the build, and its table goes into the bundle as EMBERHUT_WIRE_DIALECT. Unset or empty, the bundle carries
// an empty string and the extension speaks its own words.

import { build } from "esbuild";
import { cpSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { parseDialect } from "../src/dialect.ts";

const root = fileURLToPath(new URL("..", import.meta.url));
const dist = `${root}dist`;

/** The dialect's table as JSON, or "" without one. */
function wireDialect(): string {
  const file = process.env.EMBERHUT_WIRE_DIALECT ?? "";
  if (file === "") return "";
  let table;
  try {
    table = parseDialect(readFileSync(file, "utf8"));
  } catch (e) {
    throw new Error(`EMBERHUT_WIRE_DIALECT (${file}): ${e instanceof Error ? e.message : String(e)}`);
  }
  return JSON.stringify(table);
}

const dialect = wireDialect();

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });
await build({
  absWorkingDir: root,
  entryPoints: { background: "src/background/main.ts", popup: "src/popup/popup.ts" },
  outdir: "dist",
  bundle: true,
  format: "iife",
  target: "chrome125",
  charset: "utf8",
  legalComments: "none",
  logLevel: "warning",
  define: { EMBERHUT_WIRE_DIALECT: JSON.stringify(dialect) },
});
cpSync(`${root}manifest.json`, `${dist}/manifest.json`);
cpSync(`${root}static/popup.html`, `${dist}/popup.html`);
cpSync(`${root}icons`, `${dist}/icons`, { recursive: true });

for (const bundle of ["background.js", "popup.js"]) {
  if (readFileSync(`${dist}/${bundle}`, "utf8").includes(root.replace(/\/$/, ""))) {
    throw new Error(`dist/${bundle} holds this machine's path to the package`);
  }
}
