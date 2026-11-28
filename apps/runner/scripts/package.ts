// Builds the runner's download for one platform: a folder with a pinned Node (from nodejs.org, its checksum checked),
// the runner's code and its production dependencies (the Agent SDK's binary for that platform among them), and a
// launcher; archived as emberhut-runner-<version>-<target>.tar.gz (a .zip for Windows) with its SHA-256 beside it.
//
//   node scripts/package.ts --target linux-x64 --version 1.2.3 [--out dist]
//
// It runs on the platform it builds for (CI's matrix does), so the dependencies' native parts are that platform's.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, cpSync, createReadStream, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { arch, platform } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

/** The targets a download is built for, and their names at nodejs.org. */
export const TARGETS = {
  "linux-x64": { os: "linux", cpu: "x64", node: "linux-x64", archive: "tar.xz" },
  "linux-arm64": { os: "linux", cpu: "arm64", node: "linux-arm64", archive: "tar.xz" },
  "darwin-arm64": { os: "darwin", cpu: "arm64", node: "darwin-arm64", archive: "tar.gz" },
  "darwin-x64": { os: "darwin", cpu: "x64", node: "darwin-x64", archive: "tar.gz" },
  "windows-x64": { os: "win32", cpu: "x64", node: "win-x64", archive: "zip" },
} as const;
export type Target = keyof typeof TARGETS;

const here = dirname(fileURLToPath(import.meta.url));
const pkgDir = resolve(here, "..");
const repo = resolve(pkgDir, "../..");
const say = (s: string) => process.stdout.write(s + "\n");
// Windows' own bsdtar: Git for Windows puts GNU tar first on the PATH, which reads "D:\\…" as a remote host.
const TAR = platform() === "win32" ? join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe") : "tar";
// On Windows pnpm is a .cmd shim, which only a shell starts.
const sh = (cmd: string, args: string[], cwd?: string) =>
  execFileSync(cmd, args, { stdio: "inherit", ...(cwd ? { cwd } : {}), shell: platform() === "win32" && cmd === "pnpm" });

async function sha256(file: string): Promise<string> {
  const h = createHash("sha256");
  for await (const c of createReadStream(file)) h.update(c as Buffer);
  return h.digest("hex");
}

/** A file from the web, tried three times. */
async function get(url: string): Promise<Buffer> {
  for (let i = 1; ; i++) {
    try {
      const r = await fetch(url, { signal: AbortSignal.timeout(300_000) });
      if (!r.ok) throw new Error(`${url}: ${r.status}`);
      return Buffer.from(await r.arrayBuffer());
    } catch (e) {
      if (i >= 3) throw e;
      await new Promise((ok) => setTimeout(ok, 2_000 * i));
    }
  }
}

/** The Node release named in .nvmrc, for the target, checked against nodejs.org's SHASUMS256.txt, unpacked to `into`. */
async function node(target: Target, work: string, into: string): Promise<string> {
  const version = readFileSync(join(repo, ".nvmrc"), "utf8").trim().replace(/^v/, "");
  const t = TARGETS[target];
  const name = `node-v${version}-${t.node}`,
    file = `${name}.${t.archive}`;
  const base = `https://nodejs.org/dist/v${version}`;
  const local = join(work, file);
  if (!existsSync(local)) writeFileSync(local, await get(`${base}/${file}`));
  const sums = (await get(`${base}/SHASUMS256.txt`)).toString("utf8");
  const want = sums
    .split("\n")
    .find((l) => l.endsWith(`  ${file}`))
    ?.split(/\s+/)[0];
  const got = await sha256(local);
  if (!want || want !== got) throw new Error(`${file}: checksum ${got} isn't nodejs.org's ${want ?? "(none listed)"}`);
  sh(TAR, ["-xf", local, "-C", work]);
  mkdirSync(dirname(into), { recursive: true });
  renameSync(join(work, name), into);
  // Only the runtime: npm, corepack and the headers aren't needed to run.
  for (const p of [
    "include",
    "share",
    "lib/node_modules",
    "node_modules",
    "npm",
    "npm.cmd",
    "npx",
    "npx.cmd",
    "corepack",
    "corepack.cmd",
    "bin/npm",
    "bin/npx",
    "bin/corepack",
  ])
    rmSync(join(into, p), { recursive: true, force: true });
  return version;
}

const LAUNCH_SH = `#!/bin/sh
# The runner's launcher: this download's own Node and the runner's code, run by Node's type stripping.
here=$(dirname "$(readlink -f "$0" 2>/dev/null || realpath "$0" 2>/dev/null || echo "$0")")
exec "$here/../node/bin/node" --disable-warning=ExperimentalWarning "$here/../app/src/cli.ts" "$@"
`;
const LAUNCH_CMD = `@echo off\r\nrem The runner's launcher: this download's own Node and the runner's code.\r\n"%~dp0..\\node\\node.exe" --disable-warning=ExperimentalWarning "%~dp0..\\app\\src\\cli.ts" %*\r\n`;

async function main(): Promise<void> {
  const { values } = parseArgs({ options: { target: { type: "string" }, version: { type: "string" }, out: { type: "string" } } });
  const here_ = `${platform() === "win32" ? "windows" : platform()}-${arch()}`;
  const target = (values.target ?? here_) as Target;
  if (!(target in TARGETS)) throw new Error(`--target: one of ${Object.keys(TARGETS).join(", ")}`);
  if (target !== here_)
    throw new Error(`build ${target} on that platform (this is ${here_}): its dependencies' native parts are per platform`);
  const version = (
    values.version ?? (JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8")) as { version: string }).version
  ).replace(/^v/, "");
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) throw new Error(`--version ${version} isn't a version`);
  const out = resolve(values.out ?? join(pkgDir, "dist"));
  const name = `emberhut-runner-${version}-${target}`;
  const work = join(out, `.work-${target}`),
    root = join(out, name);
  rmSync(root, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });

  say(`${name}: Node`);
  const nodeVersion = await node(target, work, join(root, "node"));

  say(`${name}: the runner and its production dependencies (from the lock file)`);
  // pnpm deploy: the package with exactly the lock file's production dependencies, in a node_modules of its own.
  // The workspace patches mobile dependencies, which this package doesn't have.
  sh(
    "pnpm",
    ["--config.allow-unused-patches=true", "--filter", "@emberhut/runner", "deploy", "--prod", "--legacy", join(work, "app")],
    repo,
  );
  mkdirSync(join(root, "app"), { recursive: true });
  for (const p of ["package.json", "src", "bin", "README.md", "node_modules"])
    cpSync(join(work, "app", p), join(root, "app", p), { recursive: true, verbatimSymlinks: true });
  for (const p of ["LICENSE-MIT", "LICENSE-APACHE"]) cpSync(join(repo, p), join(root, p));
  const pkg = JSON.parse(readFileSync(join(root, "app", "package.json"), "utf8")) as Record<string, unknown>;
  pkg.version = version;
  delete pkg.devDependencies;
  delete pkg.scripts;
  writeFileSync(join(root, "app", "package.json"), JSON.stringify(pkg, null, 2) + "\n");

  mkdirSync(join(root, "bin"), { recursive: true });
  if (TARGETS[target].os === "win32") writeFileSync(join(root, "bin", "emberhut-runner.cmd"), LAUNCH_CMD);
  else {
    writeFileSync(join(root, "bin", "emberhut-runner"), LAUNCH_SH);
    chmodSync(join(root, "bin", "emberhut-runner"), 0o755);
  }
  writeFileSync(join(root, "VERSION"), `${version}\nnode ${nodeVersion}\n`);

  say(`${name}: checking it runs`);
  const launcher = TARGETS[target].os === "win32" ? join(root, "bin", "emberhut-runner.cmd") : join(root, "bin", "emberhut-runner");
  const said = execFileSync(launcher, ["version"], { encoding: "utf8", shell: TARGETS[target].os === "win32" }).trim();
  if (said !== version) throw new Error(`the built runner says it is ${said}, not ${version}`);

  const archive = join(out, TARGETS[target].os === "win32" ? `${name}.zip` : `${name}.tar.gz`);
  rmSync(archive, { force: true });
  if (TARGETS[target].os === "win32") sh(TAR, ["-a", "-c", "-f", archive, name], out);
  else sh(TAR, ["-czf", archive, name], out);
  const sum = await sha256(archive);
  writeFileSync(`${archive}.sha256`, `${sum}  ${archive.split(/[/\\]/).pop()!}\n`);
  rmSync(work, { recursive: true, force: true });
  say(`${archive}\n${sum}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  main().catch((e: unknown) => {
    process.stderr.write(`package: ${e instanceof Error ? e.message : String(e)}\n`);
    process.exit(1);
  });
