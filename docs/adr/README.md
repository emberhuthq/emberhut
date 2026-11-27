# Architecture decision records

One file per decision, numbered. Once accepted, a decision is never rewritten: a later ADR
supersedes an earlier one and says so. Until the first release, wording and naming fixes to an
accepted ADR are allowed; a changed decision still needs a new ADR. Format:
[MADR](https://adr.github.io/madr/) light: Context, Decision, Consequences.

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-workspace-and-quality-bar.md) | Workspace layout and quality bar | Accepted |
| [0002](0002-rust-server.md) | The server: axum, SeaORM on SQLite, opaque device tokens | Proposed |
| [0003](0003-monorepo-layout.md) | Monorepo layout: Rust, mobile, web and shared packages | Accepted |
| [0005](0005-releases-and-updates.md) | Releases and updates: semver and build, signed on the Mac, a private feed | Proposed |
| [0006](0006-report-webview.md) | Reports in a web view: gpui-webview, on macOS only | Proposed |
| [0007](0007-connectivity.md) | Connectivity: direct routes first, our relay last, and account sign-in | Accepted |
| [0008](0008-end-to-end-encryption.md) | End-to-end encryption: sealed sessions between devices and machines | Proposed |
| [0009](0009-mcp-settings-and-state-groundwork.md) | The MCP endpoint, the settings actions and the state's sections: one registry each | Proposed |
| [0010](0010-chat-files-history-and-health.md) | The chat's files, history pages, and the server's health | Proposed |
| [0011](0011-push-sign-in-status-and-secret-links.md) | Push, the sign-in status and secret links; no Sign in with Apple | Proposed |
| [0012](0012-mac-helper-backup-and-builds.md) | The Mac helper, the backup and the newest builds; no Sparkle feed | Proposed |
| [0013](0013-coding-thread-app.md) | The coding-thread app: followed over MCP, polled into the threads table | Proposed |
| [0014](0014-agent-runtime-and-wake-rule.md) | The agent runtime: a CLI session per room, the wake rule, and the assistant's own tools | Proposed |
| [0015](0015-tasks-projects-and-plan.md) | Tasks, projects, areas and the plan: one transaction per change, rules in the core | Proposed |
| [0016](0016-memory-and-recall.md) | Memory: facts in files, quick notes, learning from corrections, and recall over FTS5 and vectors | Proposed |
| [0017](0017-runners-machines-and-accounts.md) | Runners on other machines: their sessions, machines, accounts and jobs | Proposed |
| [0018](0018-workers-ultra-and-guard-rails.md) | Workers, ultra runs and the guard rails: a CLI session per task, a gate per step, one OK per job | Proposed |
| [0019](0019-calls-voice-calendar-and-health.md) | Calls, voice, the calendar, and health from the owner's devices | Proposed |
| [0020](0020-messaging.md) | Messaging: Slack, Gmail, voice notes, the assistant's own WhatsApp number, and sending as the owner behind an OK | Proposed |
| [0021](0021-browser-chrome-and-mac-tools.md) | The browser the assistant drives, the owner's Chrome profiles and the Mac tools | Proposed |
| [0022](0022-side-apps.md) | The side apps: parties, the off-the-record room and family members' rooms | Proposed |
| [0023](0023-browser-extension.md) | The browser extension: in the repository, in TypeScript, its own protocol, updated from the owner's server | Proposed |
| [0024](0024-mac-helper.md) | Emberhut Hands: the Mac helper as its own app, in Rust over objc2 | Proposed |
| [0027](0027-agents-and-approvals.md) | Agents and approvals: configured assistants, one approval mechanism, and a fixed core | Proposed |
| [0028](0028-agent-model-providers.md) | The agent's model provider: chosen from the apps, handed to the CLI in its environment | Proposed |
| [0029](0029-openai-bridge.md) | The OpenAI bridge: Anthropic's Messages API on loopback, answered by a Chat Completions endpoint | Proposed |
