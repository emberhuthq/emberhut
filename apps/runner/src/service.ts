// Running the runner as a service of the logged-in user, so it starts at login and comes back after a crash: a
// systemd user unit on Linux, a launchd agent on macOS, a scheduled task at logon on Windows. It runs as the user (not
// as root or a system service) because the agent's logins, the macOS Keychain and the user's folders are theirs.
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, platform } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { Config } from "./config.ts";

/** The line every file this writes starts with, so it never replaces a file it didn't write. */
const MARK = "Written by emberhut-runner";
/** The service's name: the systemd unit, the launchd label and the scheduled task. */
export const SERVICE = { unit: "emberhut-runner", label: "com.emberhut.runner", task: "Emberhut Runner" } as const;

/**
 * The command a service runs: the installed launcher when the installer named one (`EMBERHUT_RUNNER_LAUNCHER`, a path
 * that stays the same across upgrades), else this Node and this package's entry; then `run`.
 */
export function command(env: NodeJS.ProcessEnv = process.env): { node: string; args: string[] } {
  const launcher = env.EMBERHUT_RUNNER_LAUNCHER;
  if (launcher) return { node: launcher, args: ["run"] };
  const entry = fileURLToPath(new URL("./cli.ts", import.meta.url));
  return { node: process.execPath, args: ["--disable-warning=ExperimentalWarning", entry, "run"] };
}

/** The PATH a service runs with: the installing shell's, with the usual system folders after it. */
export function servicePath(env: NodeJS.ProcessEnv = process.env): string {
  const sys =
    platform() === "darwin"
      ? ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"]
      : ["/usr/local/bin", "/usr/bin", "/bin"];
  return [...new Set([...(env.PATH ?? "").split(":").filter((p) => p.startsWith("/")), ...sys])].join(":");
}

/** The runner's log file, where every service writes. */
export const logFile = (c: Pick<Config, "home">): string => join(c.home, "runner.log");

const quote = (s: string) => (/[\s"'\\$`]/.test(s) ? `"${s.replace(/(["\\$`])/g, "\\$1")}"` : s);
const xml = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const run = (cmd: string, args: string[]) => execFileSync(cmd, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
const tryRun = (cmd: string, args: string[]) => spawnSync(cmd, args, { encoding: "utf8" });

function writeOwn(file: string, text: string, force: boolean): void {
  if (existsSync(file) && !readFileSync(file, "utf8").includes(MARK) && !force)
    throw new Error(`${file} exists and wasn't written by emberhut-runner; move it away or pass --force`);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, text);
}

/** The systemd user unit's text. */
export function systemdUnit(c: Pick<Config, "home">): string {
  const { node, args } = command();
  return [
    `# ${MARK}. Reinstall with: emberhut-runner service install`,
    "[Unit]",
    "Description=Emberhut runner (runs coding-agent sessions for an Emberhut server)",
    "After=network-online.target",
    "",
    "[Service]",
    `ExecStart=${[node, ...args].map(quote).join(" ")}`,
    `Environment=EMBERHUT_RUNNER_HOME=${quote(c.home)}`,
    // Sessions find the same tools (git, compilers, package managers) as the shell the service was installed from.
    `Environment=PATH=${quote(servicePath())}`,
    `WorkingDirectory=${quote(c.home)}`,
    "Restart=on-failure",
    "RestartSec=5",
    // Running turns resume at the next start; a stop needn't wait for them.
    "TimeoutStopSec=15",
    `StandardOutput=append:${logFile(c)}`,
    `StandardError=append:${logFile(c)}`,
    "",
    "[Install]",
    "WantedBy=default.target",
    "",
  ].join("\n");
}

/** The launchd agent's property list. */
export function launchdPlist(c: Pick<Config, "home">): string {
  const { node, args } = command();
  return `<?xml version="1.0" encoding="UTF-8"?>
<!-- ${MARK}. Reinstall with: emberhut-runner service install -->
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>${SERVICE.label}</string>
  <key>ProgramArguments</key>
  <array>
${[node, ...args].map((a) => `    <string>${xml(a)}</string>`).join("\n")}
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>EMBERHUT_RUNNER_HOME</key><string>${xml(c.home)}</string>
    <key>PATH</key><string>${xml(servicePath())}</string>
  </dict>
  <key>WorkingDirectory</key><string>${xml(c.home)}</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>5</integer>
  <key>StandardOutPath</key><string>${xml(logFile(c))}</string>
  <key>StandardErrorPath</key><string>${xml(logFile(c))}</string>
</dict>
</plist>
`;
}

/** The Windows wrapper the scheduled task starts: the runner, its output appended to the log. */
export function windowsScript(c: Pick<Config, "home">): string {
  const { node, args } = command();
  return [
    `@rem ${MARK}.`,
    `@set "EMBERHUT_RUNNER_HOME=${c.home}"`,
    `@call "${node}" ${args.map((a) => `"${a}"`).join(" ")} >> "${logFile(c)}" 2>&1`,
    "",
  ].join("\r\n");
}

const unitFile = () => join(process.env.XDG_CONFIG_HOME ?? join(homedir(), ".config"), "systemd", "user", `${SERVICE.unit}.service`);
const plistFile = () => join(homedir(), "Library", "LaunchAgents", `${SERVICE.label}.plist`);
const uid = () => process.getuid?.() ?? 0;

/** Installs and starts the service. Returns what it did, in words. */
export function install(c: Config, force = false): string {
  mkdirSync(c.home, { recursive: true, mode: 0o700 });
  switch (platform()) {
    case "linux": {
      writeOwn(unitFile(), systemdUnit(c), force);
      run("systemctl", ["--user", "daemon-reload"]);
      run("systemctl", ["--user", "enable", `${SERVICE.unit}.service`]);
      run("systemctl", ["--user", "restart", `${SERVICE.unit}.service`]);
      const linger = tryRun("loginctl", ["show-user", process.env.USER ?? "", "-p", "Linger"]).stdout?.trim();
      return `systemd user unit ${unitFile()} enabled and started${linger === "Linger=no" ? ". To keep it running while you're logged out: loginctl enable-linger" : ""}`;
    }
    case "darwin": {
      writeOwn(plistFile(), launchdPlist(c), force);
      tryRun("launchctl", ["bootout", `gui/${uid()}/${SERVICE.label}`]);
      run("launchctl", ["bootstrap", `gui/${uid()}`, plistFile()]);
      return `launchd agent ${plistFile()} loaded`;
    }
    case "win32": {
      const script = join(c.home, "run-runner.cmd");
      writeOwn(script, windowsScript(c), force);
      run("schtasks", ["/Create", "/F", "/SC", "ONLOGON", "/RL", "LIMITED", "/TN", SERVICE.task, "/TR", `"${script}"`]);
      run("schtasks", ["/Run", "/TN", SERVICE.task]);
      return `scheduled task "${SERVICE.task}" (at logon) created and started`;
    }
    default:
      throw new Error(`no service manager known for ${platform()}: run "emberhut-runner run" under your own`);
  }
}

/** Stops and removes the service (the runner's home, accounts and sessions stay). */
export function uninstall(c: Config): string {
  switch (platform()) {
    case "linux":
      tryRun("systemctl", ["--user", "disable", "--now", `${SERVICE.unit}.service`]);
      if (existsSync(unitFile()) && readFileSync(unitFile(), "utf8").includes(MARK)) rmSync(unitFile());
      tryRun("systemctl", ["--user", "daemon-reload"]);
      return "systemd user unit stopped and removed";
    case "darwin":
      tryRun("launchctl", ["bootout", `gui/${uid()}/${SERVICE.label}`]);
      if (existsSync(plistFile()) && readFileSync(plistFile(), "utf8").includes(MARK)) rmSync(plistFile());
      return "launchd agent unloaded and removed";
    case "win32":
      tryRun("schtasks", ["/End", "/TN", SERVICE.task]);
      tryRun("schtasks", ["/Delete", "/F", "/TN", SERVICE.task]);
      rmSync(join(c.home, "run-runner.cmd"), { force: true });
      return "scheduled task removed";
    default:
      return "nothing to remove";
  }
}

/** Restarts the service. */
export function restart(): string {
  switch (platform()) {
    case "linux":
      run("systemctl", ["--user", "restart", `${SERVICE.unit}.service`]);
      return "restarted";
    case "darwin":
      run("launchctl", ["kickstart", "-k", `gui/${uid()}/${SERVICE.label}`]);
      return "restarted";
    case "win32":
      tryRun("schtasks", ["/End", "/TN", SERVICE.task]);
      run("schtasks", ["/Run", "/TN", SERVICE.task]);
      return "restarted";
    default:
      throw new Error(`no service manager known for ${platform()}`);
  }
}

/** What the service manager says of it. */
export function status(): string {
  const r =
    platform() === "linux"
      ? tryRun("systemctl", ["--user", "status", "--no-pager", "--lines=0", `${SERVICE.unit}.service`])
      : platform() === "darwin"
        ? tryRun("launchctl", ["print", `gui/${uid()}/${SERVICE.label}`])
        : tryRun("schtasks", ["/Query", "/TN", SERVICE.task, "/V", "/FO", "LIST"]);
  return (r.stdout || r.stderr || "not installed").trim().split("\n").slice(0, 12).join("\n");
}
