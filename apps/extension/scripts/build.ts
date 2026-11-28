// Builds the extension into dist/: the service worker, the popup and the consent page bundled by esbuild (not minified,
// no source maps: the store reads the code as it runs), with the manifest, the pages and the icons beside them. dist/ is the folder Chrome loads unpacked, and what scripts/zip.ts packs. Paths in
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
import { versionProblem } from "./release.ts";

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

// The extension ships with the apps: its version is the workspace's (Cargo.toml), in the manifest and package.json alike.
const problem = versionProblem(
  readFileSync(`${root}manifest.json`, "utf8"),
  readFileSync(`${root}package.json`, "utf8"),
  readFileSync(`${root}../../Cargo.toml`, "utf8"),
);
if (problem) throw new Error(problem);

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });
await build({
  absWorkingDir: root,
  entryPoints: { background: "src/background/main.ts", popup: "src/popup/popup.ts", consent: "src/consent/consent.ts" },
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
cpSync(`${root}static/consent.html`, `${dist}/consent.html`);
cpSync(`${root}icons`, `${dist}/icons`, { recursive: true });

for (const bundle of ["background.js", "popup.js", "consent.js"]) {
  if (readFileSync(`${dist}/${bundle}`, "utf8").includes(root.replace(/\/$/, ""))) {
    throw new Error(`dist/${bundle} holds this machine's path to the package`);
  }
}
