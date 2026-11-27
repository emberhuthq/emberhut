# 0027. Agents and approvals: configured assistants, one approval mechanism, and a fixed core

- Status: Proposed
- Date: 2026-11-26
- Supersedes: the "second agent with its own persona" item of [ADR 0014](0014-agent-runtime-and-wake-rule.md)'s
  "What is not ported", and the "work handed for another person's OK" and "the list of people whose
  asks count as the owner's" items of [ADR 0018](0018-workers-ultra-and-guard-rails.md)'s
  "Not ported, and why".

## Context

The server runs one assistant for its owner. Its OKs are the owner's alone, and the guard rails
(ADR 0018) decide when a step needs one: one job, one OK; a message or post's exact text OK'd
once; payments, passwords, codes and account security asked every time.

An installation also wants assistants that work for someone else on the owner's behalf: an
assistant that serves a client in the client's Slack, with its own persona and room, its own
tools (the client's systems, reached through MCP servers), its own memory, and a budget. Such an
assistant's outward steps (a public post, a merge, a deploy) are the client's to approve, not
the owner's, and the client approves them from wherever they are: a direct message, or a link
on their phone, without an account on this server. Some of what it could touch is never the
client's to approve: money, secrets, account security, and anything said in the owner's name.

The reference server did this with code written for one client: a module per client with its
own room, tool set, Slack bridge, approvals table, a check against the client's own admin app,
and a hard-coded approver id. Those parts were not ported (ADR 0014, ADR 0018). A list in the
configuration made some people's requests count as the owner's own; it was not ported either.

This ADR replaces all of it with three general things: **agents**, **approvals**, and **policies**,
around a **fixed core** that nothing configurable can loosen. Integrations with any one client's
systems are MCP servers outside this repository.

## Decision

### Agents

An agent is a configured assistant. Its row holds:

- `id`, a slug (`[a-z0-9][a-z0-9-]{0,39}`), and `name`, as the apps show it;
- `persona`: its system prompt, put where the server's own rules go for the default agent;
- `principal`: who it works for, a display name only ("the owner" for the default agent); it
  decides nothing by itself, the policies do;
- `room_id`: its own room. Every room belongs to exactly one agent, the default agent's being
  every room without an agent row;
- `project_id`: the project it is scoped to, whose facts it reads and never writes; none: no
  project facts;
- `spend_cap_cents` and `spend_currency`: the most its approved spending may add up to in a
  calendar month (none: no spending at all);
- `enabled`: a disabled agent's room takes no turns, and it neither asks for nor uses an
  approval, posts, nor gives its tasks work; its workers start no more and run nothing
  (disabling is the owner's off switch, and what an agent that did anything gets instead of
  deletion);
- `worker_shell`: whether its tasks' workers may run shell commands (off by default). A shell
  runs as the server's user, and the gate sees only its command line, so turning it on gives up
  the agent's roots: such a worker can read any file the server's user can read and copy it into
  its task's folder, where `agent_reply` and an approval may take it from; it can reach the
  network (a read is not an outward step), read the server's environment, and plant links the
  file tools then meet. Only the steps the outward rule recognises (a merge, a deploy) still wait
  on the agent's policies. It is for an agent the owner trusts with everything the server's user
  can reach; the default agent's workers are the owner's own and never this.

The **default agent** is the assistant the server already runs: id `main`, made by the
migration, its room Main and every room no other agent owns. It has no persona of its own (the
server's rules), every tool (ADR 0009's `room_tools`), and an implicit policy that every
approval is the owner's, which is how the guard rails already behave. Nothing about it changes.

What an agent has besides, each its own table so it is queried and changed row by row:

- **Channels** (`agent_channels`): `kind` (`slack`, `whatsapp`), `workspace` (a Slack team id;
  none for WhatsApp), `address` (a channel id, or a WhatsApp chat id), and for Slack the name of
  the secret holding the bot token (`token_secret`, a file in the secrets directory, read at every
  use). Its room is always one of its channels.
- **Tools** (`agent_tools`): `kind` `server` (one of the server's own MCP tools by name, from
  `room_tools`) or `mcp` (an MCP server of its own: `name` and `config`, the path of a JSON file
  outside the repository giving the server's command or URL, either the server's own object or an
  `mcpServers` object naming exactly one, added to its session's `--mcp-config` and to
  `--allowedTools` as `mcp__<name>__*`; a file that can't be read, a server taking the server's own
  MCP name, and a file the server's user doesn't own, others may write or in the workers' folders
  (whoever writes it chooses what the session runs) are left out and logged). An agent other than
  the default gets only what is listed here, plus the agent tools below; the tools that act as the
  owner (`slack_send`, `whatsapp_send`, the browser's and the Mac's tools, `messages_*`, `gmail_*`,
  `health`, `screen_time`, `calendar`) are refused as server tools for any agent but the default, at
  write time and again when its tool set is built. Of the server tools it lists, only those that act
  for it alone are served: the ones that act in its own room (`wake_me_at`, `watch`, `send_file`)
  and its task tools, scoped to the tasks made in its room (`tasks`, `task_create`, `task_send`,
  `task_stop`, `task_cancel`). A tool that reads or changes the owner's own things (their messages,
  plan, notes, other rooms, every task, recall over everything) is not served to it, and the server
  logs that it was listed.
- **Accounts** (`agent_accounts`): `label` and `secret` (the secret file's name). The server
  never shows a secret; an account is how an agent's MCP server is told which credentials to use,
  through its environment (`EMBERHUT_AGENT_ACCOUNT_<LABEL>` holding the file's path, never its
  contents).
- **Memory.** Its own facts are the facts whose category is `agent-<id>` (a dash, so the
  category's folder has the same name on every file system); its `facts` tool
  lists those and, read-only, its project's (the owner's facts kept for that project);
  `fact_set` and `fact_drop` change only its own, and the owner's `facts` never list an
  agent's. Recall over everything is not served to an agent; a recall scoped to its room's
  messages, its tasks, its own facts and its project's is left for later.

An agent's room session is ADR 0014's, launched apart from the owner's setup: the system prompt
is its name and principal, its persona, then the server's fixed-core rules (below), none of the
owner's own rules or configured prompt; `--strict-mcp-config` with only the server's endpoint
(which answers only its tool set) and its own MCP servers; no setting sources
(`--setting-sources` empty: none of the owner's settings, memory or hooks); no built-in tools
(`--tools` empty: it acts only through MCP tools); always `dontAsk`; none of the installation's
extra arguments; and its own empty working folder, `<data dir>/agents/<id>`, made for the
server's user only. The account's configuration folder is still where its sign-in is. A
disabled agent's room takes no turns and never gets a launch. The setup has a fingerprint (the
agent and a hash of its prompt, servers, accounts and folder): a session open or kept under
another (another agent on the room, or the same agent changed) is closed and its conversation
not resumed, so an agent given an existing room starts afresh. No other room's words reach an
agent's turns (the same-task lines other rooms said about a task key are left out), and neither
does the owner's own messaging: an event or chat line of it meant for a room an agent was given
(a WhatsApp chat bound to that room, or the WhatsApp room itself) goes to Main instead, and a
chat's asks cannot be pointed at an agent's room.

An agent's tasks are made in its room, the room written with the task in one insert; a subtask
or a finding filed under one is made in the same room, so it is the agent's too; who asked is
kept for showing (`requested_by`, as the agent wrote it) and grants nothing; their history names
the agent, and every guarded step of theirs is decided by the agent's policies. A worker on one
runs apart from the owner's setup too: no setting sources, none of the installation's extra
arguments, `dontAsk`, its task's folder as its working folder, and only the file tools (`Read`,
`Glob`, `Grep`, `Write`, `Edit`, plus `Bash` when the agent's `worker_shell` allows), each behind
the gate. The gate keeps the file tools to the agent's roots (its own folder and its tasks' work
folders, as real paths): every field of theirs that names a file is checked, a file's path and a
search's folder, and also Glob's `pattern` and Grep's `glob`, since a search starts from an
absolute pattern's own fixed folder whatever folder it was given (a relative one starts from the
search's folder); a pattern part that steps up (`..`) or starts at a home folder, a link inside
the roots that leads nowhere yet (writing through it could land anywhere), a file field that
isn't text or is missing, and an unknown field that names a file or holds an absolute path are
refused. The shell goes through the outward rule and the agent's policies. Its MCP tools are its
own task's, the agent's approval tools and, read-only, the agent's facts, in the agent's room;
none of the owner's reads (their messages, calendar, plan, recall); it writes only on the
agent's own tasks, whatever its brief names. Everything the server tells about an agent's task
(what its run ends with, a task that didn't start, an ask that waited on the owner for 12 hours,
the owner's answer, message or change on it: done, archived, unarchived, a new description) is
told in the agent's room, in a line naming only the task and the agent's own tools, never in
Main; for a task no one owns, nobody is told. So the owner's assistant is never told of an
agent's work, and nothing an agent or its worker wrote reaches the owner's session: the owner
hears of it through Needs you (its asks), approvals and the apps' task lists. A task made in a
room no agent owns now is no one's: no tools, no worker, every step held, nobody told; it never
becomes the owner's. A disabled agent's task, or no one's, is passed over before a message
queued for its worker is taken, so the message waits for the agent to be enabled again.

Messages reach an agent through `POST /api/agents/{id}/inbound`: a bridge (any program that holds
the channel's connection, such as a Slack Socket Mode client) passes on a message from one of the
agent's channels, with the bearer token in the secret `agent-<id>-inbound`, compared in constant
time and checked before the body is read (no secret saved: nobody gets in; the default agent
takes no bridge, 404). The body is
`InboundMessage`: the channel's `kind`, `workspace`, `channel` and `channelName`, the `thread`,
the message's `ts`, who said it (`from`, and the channel's `user` id), the `text`, the `files`
shared (their ids and names), and whether it `mention`s the agent or is a `direct` message;
each field shown in the header (all but the words) is one line with no control characters. A
message from a channel the agent does not have is refused (403); a direct message to its bot
counts as from its channel when the agent has a Slack channel in that workspace. A direct
message, a mention, or a reply in a thread the agent is in becomes a turn in the agent's room,
framed as a channel message and never as `Event:` (the server's word): one header line,
`[Slack] "<who>" · <channel> · <thread>` with how to answer there, every name anyone can set (who
said it, the channel's name, a file's name) in double quotes with `"` and `\` escaped, and an id
shown plain only when it holds only an id's characters, so a display name such as
`Example · direct message D0OTHER` cannot change where or how the agent answers; then the words
and the files, every line quoted with `> `, so no line of the person's can pass for the server's
or for a header. A line ends at every break a model or a terminal reads as one (`\n`, `\r`,
`\r\n`, a vertical tab, a form feed, U+0085, U+2028 and U+2029), and every other control
character but the tab is a space (`emberhut_core::agents::quote`, the one quoting of outside text the
agent's room and the `agents` tool share). The fixed core tells the agent that `<who>` is the
name the channel shows, which anyone there can set, and proves nobody's identity.
Once it is taken the thread is remembered (`agent_threads`: channel, thread, its Slack team,
last seen, last speaker) so a reply in a thread it is in reaches it without a mention; anything
else is taken and left. The
answer is `{ turn, skipped }`; a disabled agent's message is left. When the room already has its
most turns waiting, the message waits as an urgent event. Nothing that arrives this way is ever
an approval: a bridge's word for who said "yes" is not proof (see the threat model). The agent
answers with `agent_reply`, posted with the channel's bot token, only into a conversation it is
in (one of its channels, or a thread it is in; a direct message is answered with the bot token
of its Slack channel in that conversation's workspace, and refused when it has none there), as
plain text: Slack's `&`, `<` and `>` escaped, `mrkdwn` and media previews off, no blocks, so it
makes no mention, link or button of its own and nothing it posts can pass for an approval's
buttons. Its files, like the files of its approval requests and uses, come only from its roots
(its own folder and its tasks' work folders, checked on the open file's real path) and are read
as an approval's are (absolute paths, no private or hidden file). A WhatsApp chat
is answered through its bridge, not `agent_reply`. A disabled agent posts nothing.

### Approvals

**Kinds of action.** Every approval is for one kind:

- configurable: `post` (publish: a social post, a comment, a public page), `send` (a message to
  people outside the agent's own conversations), `merge`, `deploy`, `create` (an account, a record
  or a resource in another system), `other`;
- **the fixed core**: `payment` (money moves or is committed: a payment, a purchase, a
  subscription, a top-up), `credential` (a password, a code, a one-time code, a key), `secret`
  (reading or handing over a secret), `account_security` (sign-in, recovery, two-factor, closing
  an account, permissions), and `as_owner` (anything said or sent in the owner's name).

**Policies** (`approval_policies`): one row per agent and kind, naming the approver: `owner`, an
external approver by id, or `none` (no approval needed). No row: `owner`.

**Approvers** (`approvers`): `id`, `name`, and how they are reached and recognised:
`slack_workspace` and `slack_user` (their Slack team and user id), `webhook_url` and
`webhook_secret` (the name of a secret file: the key both sides sign with), and whether a signed
link may be sent to them (`link`). The owner is not a row: the owner approves in the apps, from a
signed-in device.

**The fixed core** is code (`emberhut_core::approvals::fixed_core`), not data:

- a fixed-core kind is the owner's, whatever the policy says: a policy that names anyone else, or
  `none`, for a fixed-core kind is refused when written and ignored if it is in the table anyway
  (an older build, an import, a hand edit);
- a request is classified by its kind and also by its words: a `post`, `send`, `create` or
  `other` whose text or summary is about money, a secret, a password or code, or account
  security (the same word list the guard rails use, plus the words for secrets and money words
  only approvals use: a subscription, a top-up, a transfer, a wire, a refund, a renewal, a
  charge, and an amount such as `$500` or `1,200 EUR`) is raised to the
  matching fixed-core kind. A `merge` or `deploy` is not raised by its words: its summary
  describes code, which is often about payments or passwords without moving money or handing
  one over;
- a spend needs a `payment` request with its amount, and the agent's month's spending plus the
  amount must stay within its cap; the cap can only refuse, never approve;
- an agent never sends as the owner: `as_owner` requests are always the owner's, and the tools
  that send as the owner are not in any agent's tool set but the default's.

**A request** (`approval_requests`) carries exactly what is to happen:

- `agent_id`, `kind` (after the fixed core's raise), `target` (where it goes: a channel, a
  repository and branch, a URL, an account; free text, part of what is approved), `summary` (one
  line of the agent's), `text` (the exact words, if any), and its files (`approval_files`: name,
  size and SHA-256 of the bytes, read by the server from the absolute path the agent gave, never
  trusted from the agent; at most 20 files of 100 MB each; a private file (the secrets folder,
  the server's keys, the owner's key folders), a hidden one, a system file (under `/proc`, `/sys`
  or `/dev`), an empty one or one with other names (hard links) is refused; the file is opened
  once and these checks run on the real path of the open handle, and what is sent to the
  approver is read again from that handle and must still have the hash, so a link swapped in
  meanwhile changes nothing);
- `payload_hash`: SHA-256 over a canonical encoding of the version tag `emberhut-approval-v1`, the
  agent, the kind, the target, the text, the amount, and the files sorted by their hash (each
  file's hash, size and name), each length-prefixed, so no two different payloads share an
  encoding. The file's name is covered because the approver is shown it and may judge by it,
  so a renamed file is a new request; a payment's amount is covered because it is what the
  approver says yes to. The summary is not covered: it is the agent's description of the step,
  not what goes out, and is shown as such (the page says its answer covers the target, text,
  amount and files); two asks for the same payload with different summaries are one request,
  which keeps the first summary;
- `approver` as the policy named it when the request was made (`owner` or the approver's id),
  `amount_cents` for a payment, `task_id` when a task's step asked;
- its state: `pending`, then `approved` or `rejected` (with `decided_at`, `decided_by`, the
  approver's identity as the channel proved it, and `decided_via`: `app`, `slack`, `link` or
  `webhook`), then `used` (with `used_at` and `used_by`, the task or tool that used it); or
  `expired` (unanswered after 3 days, or approved and unused after 24 hours) or `withdrawn` (the
  agent or the owner took it back).

A request with the same agent, kind and payload hash as one still pending is that request, not a
new one. A changed word, file or target is a different hash: a new request. A payment over what
the month's cap leaves is refused when it is asked for, and again when it is used.

**Using an approval** is one conditional update, in one transaction: the row whose agent, kind
and payload hash match the step's (and whose id is the one the step names, when it names one),
`approved`, unused, not expired, and whose approver is still the one the agent's policy names
now, becomes `used`. Zero rows changed: nothing goes out. The
payload is hashed again from the step itself (its text, and the bytes of its files read now), so
a file edited after it was approved does not match.

**Who approves, and how they are recognised** (the channel adapters,
`services::approvals::channels`, each a trait with a real implementation and a stand-in):

- **The owner**, in the apps: `GET /api/approvals` lists what waits on them (and what waits on
  others, for information), `POST /api/approvals/{id}/decision` with
  `{ "decision": "approve" | "reject", "payloadHash": "..." }` from a signed-in device, in either
  auth mode. A fixed-core request is only ever decided this way.
- **Slack DM** (`SlackDm`): the server opens a DM with the approver's Slack user through the
  agent's bot token, uploads the files, and posts the text word for word with Approve and Reject
  buttons whose value is the request id and payload hash (`<id>.<hash>`). Slack calls
  `POST /api/approvals/slack` (interactivity); the request is checked with the workspace's
  signing secret (`slack-<team>-signing`, Slack's `v0` HMAC over the timestamp and body, at most
  five minutes old), and counts only when the click's team and user are the approver's, the
  button is on the very message the server posted (its channel and timestamp, kept when it was
  posted), and the hash is the request's. The message is then updated to say what was decided,
  after the click is answered. Everything shown in the DM (and on the link's page) has its
  invisible characters (bidirectional controls, zero-width and other format characters) written
  out as `[U+XXXX]`, and a file's name may not hold one.
- **Signed link** (`SignedLink`): a one-time page `GET /approve/{token}` that needs no account.
  The token is the request id, an expiry and a random nonce, with an HMAC-SHA256 over them and
  the payload hash, keyed by the server's `approval-link-key` secret (made on first use). The
  page shows the agent, the target, the text and each file's name, size and hash, with Approve
  and Reject (`POST /approve/{token}`, which also carries the hash the page showed). The server
  stores only the nonce's hash; the first decision clears it, so a link works once. The link is
  handed to the approver's system by the webhook below, and never put in the Slack DM: the
  agent's bot is a member of that DM and could read it back, and a link needs no identity, so
  the DM's buttons (which prove the approver's Slack identity) are the way to decide there. The
  link key is written whole to a file of its own and then named, so no reader sees half a key.
- **Webhook** (`Webhook`): for an approver with an app of their own (where an approve button
  lives). When a request is made, the server `POST`s it to the approver's `webhook_url` (JSON:
  `id`, `agent`, `agentName`, `kind`, `target`, `summary`, `text`, `files` with each name, size
  and hash, `amountCents`, `payloadHash`, `expiresAt`, `link` when the approver may have one,
  and `answerUrl`, where to answer), signed `x-emberhut-timestamp` and `x-emberhut-signature: v1=<HMAC-SHA256 over
  "<ts>.POST.<path>.<body>">` with the approver's secret. Their app answers on
  `POST /api/approvals/{id}/webhook` with `{ "decision", "payloadHash", "by" }`, signed the same
  way with the same secret over that path as written here (whatever proxy is in front), at most
  five minutes old; `by` is recorded as their app reports it. Until a Slack or webhook call is
  proven, every refusal is the same 401, so an outsider learns nothing of which requests,
  approvers or workspaces exist.

A request is routed outside only when the server has its public address and the agent is
enabled; otherwise it waits in the apps (on the owner's list, for information), and asking for
it says so. A Slack DM is sent with the bot token of the agent's Slack channel in the
approver's workspace; an agent without one there reaches the approver by webhook only.

### Trusted requesters

`trusted_requesters` rows (`requester`, the name a task's request carries; `agent_id`, or none
for every agent) make that person's requests count as the owner's own request, for the job's
grant (ADR 0018): their posts and comments on that task go through without an OK per text, as
the owner's own request would. The name is the one in a default agent's task's `requested_by`
(`who`, `at`, `how`, the reference server's shape for a task asked for by someone other than
the owner, as an import keeps it). A trusted requester never applies to a task made in a
configured agent's room, whoever it names: the agent writes that name itself, so it proves
nothing, and no job grant applies to an agent's task anyway. They never cover the fixed core or
anything said in the owner's name, and they are data, changed only from a signed-in device; the
configured list of the reference server becomes rows at import.

### API, tools and the apps

Routes (all JSON, in the OpenAPI document; a signed-in device unless said otherwise, in either
auth mode: report mode's pass without a session reads the app, never these):

- `GET /api/agents`, `POST /api/agents` (create), `GET`/`PATCH`/`DELETE /api/agents/{id}`
  (the default agent cannot be deleted, renamed, given a persona or narrowed), and
  `PUT /api/agents/{id}/channels`, `/tools`, `/accounts`, `/policies`, each replacing the
  agent's whole list in one transaction;
- `GET`/`POST /api/approvers`, `PATCH`/`DELETE /api/approvers/{id}` (an approver a policy or
  an open request names is not deleted, 409: a new approver given the same id must not decide
  or use the old requests);
- `GET`/`PUT /api/trusted-requesters`;
- `GET /api/approvals?state=&agent=` (a state or agent no request has: none),
  `GET /api/approvals/{id}`,
  `POST /api/approvals/{id}/decision` (the owner), `POST /api/approvals/{id}/withdraw`;
- unauthenticated, each proving itself as above: `POST /api/agents/{id}/inbound` (the bridge's
  bearer), `POST /api/approvals/slack` (Slack's signature), `POST /api/approvals/{id}/webhook`
  (the approver's signature), `GET`/`POST /approve/{token}` (the signed link).

The state (`GET /api/state`) gains `agents` (id, name, room, pending approvals count; left out
while the default agent is the only one) and `approvals` (the pending ones; left out when there
are none; both only for a signed-in device, never report mode's pass without a session); an
agent's room is in `rooms` like any room, and the apps match it to its agent by
room id. A request's `amountCents` comes with `amountCurrency`, the agent's spending currency.
The live stream (`GET /api/live`) sends a named event, `event: approvals`, whose data is
`{ id, state, pending }`, whenever a request changes (made, decided, used, withdrawn, expired),
to a signed-in device's stream only.

MCP tools, for an agent's room and its workers (the default agent's room keeps its own and gets
these too):

- `approval_request` `{ kind, target, summary, text?, files?, amount_cents?, task? }`: makes or
  finds the request and routes it; answers its id, its approver as a role ("the owner", "the
  approver") and that the work waits;
- `approval_status` `{ id }`; `approval_withdraw` `{ id }`;
- `approval_use` `{ id, text?, files? }`: uses an approval for a step the agent's own MCP server
  does next (a publish in a client's system), answering a receipt line, or why not;
- `agent_reply` `{ channel, thread?, text, files? }`, `agent_thread` `{ channel, thread, limit? }`;
- for an agent's memory, `facts`, `fact_set` and `fact_drop` scoped as above;
- for an agent that lists them, its task tools scoped to the tasks made in its room: `tasks`
  `{ id? }`, `task_create` `{ title, brief, requested_by }`, `task_send` `{ id, text }` (to the
  task's worker, noted as the agent's and never as the person's), `task_stop` `{ id }` and
  `task_cancel` `{ id, why }`;
- for the default agent only, `agents` `{ agent?, since?, limit? }`: every configured agent's
  state (enabled, working or idle, pending approvals, open tasks), or one agent's conversation
  since a time (the last 24 hours by default), events included, every message's words quoted
  (`> `, at every line break, as an inbound message's are) as outside content, never
  instructions to the owner's assistant.

A disabled agent's `approval_request`, `approval_use`, `agent_reply`, `task_create`, `task_send`,
`task_stop` and `task_cancel` are refused. No tool's answer names the approver: only their role and
times.

An agent never sees an approver's Slack id, webhook, secret or link, nor who decided: a use is
answered with the approver's role and the time. An agent's MCP server that
acts outside (a plugin) uses an approval through `approval_use` before its step, or through
`POST /api/approvals/{id}/use` with the agent's room token.

The guard rails decide every guarded step of a task made in an agent's room by the agent's policies,
never by the owner's job grant or OKs (ADR 0018's grants, the outward OKs and a trusted requester's
request have none for it, so its workers always have the gate): a final step's Go ahead
(`step_gate`) and an outward step (merge, deploy) become approval requests of the matching kind to
the policy's approver (the owner, in the apps, for a kind it gives the owner), and go through once
on a matching approval; hard steps stay the owner's own cards, which no grant covers, and messages
as the owner go through messaging's own guard. A final step that says something (a post, a comment,
a message) is a `post`, its target the page and its text the words written; another final click is
`other`. An outward command is a `merge` when every step takes code to a repository (`git …`,
`gh pr …`) and a `deploy` otherwise, its target the repositories it touches (or its folder) and its
text the command line. The fixed core's raise applies to their words. A policy naming nobody lets the
step through; otherwise the step goes through once on the matching approval, its line naming the
approver's role and the time. A disabled agent's steps, and a task no agent owns now, wait. The
default agent's tasks are unchanged.

The desktop and mobile apps show agents as rooms (the agent's name, with its principal), a
**Pending approvals** list (what waits on the owner, with Approve and Reject on the exact
payload; what waits on others, read-only with its state), and an agent's settings (channels,
tools, policies, approvers, cap).

### Data model

One migration, `m20261127_000001_agents_and_approvals`: `configured_agents` (the agents, with the
default agent's row; named apart from `agents`, which already holds the sub-agents of tasks' runs),
`agent_channels`, `agent_tools`, `agent_accounts`, `agent_threads` (keyed on agent, channel and
thread), `approvers`, `approval_policies` (unique on agent and kind), `trusted_requesters`,
`approval_requests` (index on state, on agent and kind and payload hash; with `state_reason`, why it
was expired, withdrawn or rejected, and `source_ref`, where an imported row came from),
`approval_files` (a size may be unknown for an imported file), `agent_spend` (one row per used
payment: agent, request, amount, month). Deleting an agent deletes its lists, threads, policies and
trusted requesters; its approval requests and spending keep it (a restricting foreign key), so
approval history outlives nothing it names. An agent that did anything (any approval request, any
task made in its room, any message in its room) is refused deletion (409) and is disabled instead,
so nothing of it becomes the owner's; an agent deleted takes its empty room with it when the
room was made with it, and leaves an existing room it was given (a project's room, an imported
room) as it was. A second migration, `m20261127_000002_agent_worker_shell`, adds
`configured_agents.worker_shell` (off), `configured_agents.made_room` (whether the agent's room
was made with it; off for an agent given an existing room, or moved to another one) and
`agent_threads.workspace` (the Slack team of a conversation an agent is in). Times are milliseconds
since the epoch. Every table is a `SeaORM` entity in `emberhut-db`, reached through `AgentStore` and
`ApprovalStore`; no SQL outside the migration. The rules (validation, the fixed core, the payload
hash, the link's token, the policy's answer) are `emberhut_core::agents` and `emberhut_core::approvals`,
without I/O.

An imported request is stored as it ended (used, expired, rejected or withdrawn), with its
`source_ref`, and its hash is taken under a tag of its own (`emberhut-approval-import-v1`), so no
live step's hash ever matches it and it can never be used.

### Threat model

- **Spoofed approver.** An approval counts only as its channel proves it: a signed-in device for
  the owner; Slack's request signature, the approver's team and user, and the request's hash for
  a button; possession of a one-time token whose MAC covers the request and its hash for a link;
  the approver's own HMAC for a webhook. Words in a conversation, a bridge's report of who said
  "yes", the agent's statement, a task note or a tool argument are never approvals. The approver's
  addresses are set only by the owner and never shown to an agent, so an agent cannot point a
  request at itself.
- **Replayed OK.** A decision is recorded once (`pending` to `approved` or `rejected`, by a
  conditional update); a link's nonce is cleared by its first use; Slack and webhook calls older
  than five minutes are refused; an approval is used once, by a conditional update, and expires
  unused after 24 hours. The same payload approved again is a new request and a new decision.
- **Payload swap.** The hash is the server's, over the exact text, target and file bytes; the
  approver is shown exactly what it covers; the link's MAC and every decision carry the hash; a
  step is hashed again when it uses the approval. A changed word or file, or a file edited after
  approval, matches nothing.
- **Policy escalation.** No agent tool writes agents, policies, approvers or trusted requesters;
  only a signed-in device does. The fixed core is code, checked when a policy is written, when a
  request is classified, and when an approval is used. A request keeps the approver its policy
  named; using it also checks the policy names the same approver now, so moving a kind to the
  owner voids approvals given by someone else. An agent acts only for itself: the agent is the
  one whose room's token called, never an argument. A trusted requester covers neither the fixed
  core nor messages as the owner.
- **What is not covered**: an agent's MCP server is code the owner chose to run; if it acts
  without using an approval, the server cannot stop it. Plugins should take their credentials
  from the agent's accounts and call `approval_use` first; the server logs every use.

### Importing an existing setup

The reference server's second assistant, its approvals history and its trusted-requester list
map onto this model through `emberhut-server import --from-backup <file> --agents <file>`, a mapping
file kept outside the repository (TOML) that names, for each agent: its id, name, persona file,
principal, the source room it takes over (by the room's `person` value and its id), its facts
category, its threads
table, its approvals table and the table of used external approvals, its channels and tools;
the approvers (Slack team and user, webhook); the policies; and the trusted requesters (or the
configuration key holding them). From the archive (ADR 0004):

- the room, its messages and its facts become the agent's room and facts (`agent-<id>`); the
  agent is made on that room (`AgentStore::create` with the mapped room, not a new one), so
  deleting it later leaves the room;
- the tasks the source made in that room keep it (the archive's `issues.room`), and whose a task
  is follows its room at every read: from the import until the agent row is there they are no
  one's (no tools, no worker, every step held, nobody told), and from then on they are the
  agent's, its policies deciding their steps. Their `requested_by` is kept for showing and
  grants nothing, so a trusted requester's request no longer stands for the owner's OK on them;
  the importer makes the agent in the same run, and a source room the mapping names no agent for
  leaves its tasks no one's until the owner makes one there;
- its threads table becomes `agent_threads`;
- each approval row becomes an `approval_requests` row with its text, files and hashes, its
  decision (who, how, when) and its use, under the mapped approver; rows waiting for an answer
  become `expired` (a new request is made when the step runs again), since the old rows were
  keyed on normalised text and cannot be bound to an exact payload;
- the trusted-requester list becomes `trusted_requesters` rows;
- the tables of the client-specific integrations (an admin app's feed, a bridge's notes) are
  archived and not mapped: their replacements are the webhook and the inbound route.

Nothing in the repository names a client, an approver, a workspace or a channel; the mapping
file does.

#### The mapping file

TOML, version 1, given with `--from-backup`. It is read and checked by the same `emberhut_core`
rules as the routes (agent ids, `Agent::validate`, channels, tools, accounts, approvers, the
fixed core's check of every policy, trusted requesters) before the backup is read, so a wrong
file fails at once with nothing made. Every table refuses keys it does not know; a value holding
`FILL` is a placeholder left in and refused; a field naming a secret names a file in the secrets
folder, and one that looks like a token (a Slack token's prefix, or more than 31 letters and
digits in a row) is refused. The manifest records the file's path and SHA-256 (and those of the
file the trusted requesters were read from), and its rows as the mapping `agents-config`.

```toml
version = 1

[[approvers]]                      # each as POST /api/approvers takes it
id = "example-lead"
name = "Example Lead"
slack_workspace = "T0EXAMPLE"      # with slack_user, or neither
slack_user = "U0APPROVER"
# webhook_url = "https://admin.example.com/approvals"   # with webhook_secret, or neither
# webhook_secret = "example-lead-webhook"               # a secret's name
link = true                        # default false

[[agents]]
id = "client-a"                    # an agent's id; not main
name = "Client A helper"
principal = "Client A"
persona = "/abs/path/persona.md"   # optional, absolute: read into the agent's persona
project = "Garden"                 # optional: an archived project's name, or project_id = 3
# spend_cap_cents = 0              # with spend_currency, or neither
# worker_shell = false
# enabled = false                  # the only value taken: an imported agent is disabled

[agents.source]                    # where its old setup is in the archive
room_person = "helper"             # the one room whose person this is becomes its room...
room_id = 12                       # ...and it must be this room
facts_category = "helper"          # optional: context/<it>/ becomes context/agent-<id>/;
                                   #   never one of the owner's categories or an agent's
threads_table = "helper_threads"   # optional: the threads layout
approvals_table = "helper_oks"     # optional: the approvals layout, with the table below
used_table = "helper_ext_used"     # optional: the used layout; needs external_prefix
archive_tables = ["helper_feed"]   # kept in the archive only, named in the report
archive_settings = ["helperNames"] # likewise, settings keys
exclude_channels = ["C0LOG"]       # channels never the agent's: their threads stay archived

[agents.source.approvals]
approver = "example-lead"          # who the history rows were approved by (or owner)
external_prefix = "ext_"           # optional, with external_marker: the columns of a
external_marker = "ext"            #   decision made in the approver's own app, and its mark
kinds = { text = "post", job = "other" }   # source kind -> action kind; one each, never the
                                           # fixed core's; a question is never an approval

[[agents.channels]]                # as PUT /api/agents/{id}/channels takes them
kind = "slack"
workspace = "T0EXAMPLE"
address = "D0EXAMPLE1"
token_secret = "client-a-slack-bot"

[agents.channels_from_threads]     # optional: each channel of its threads table, once
workspace = "T0EXAMPLE"
token_secret = "client-a-slack-bot"

[[agents.tools]]                   # kind server (by name) or mcp (name and config)
kind = "server"
name = "tasks"

[[agents.accounts]]
label = "admin"
secret = "example-admin-key"

[[agents.policies]]                # a fixed-core kind for anyone but the owner is refused
kind = "post"
approver = "example-lead"          # owner, none, or an approver above

[trusted_requesters]
names = ["Example Person"]         # or: from_config = { file = "/abs/path/config.json", key = "..." }
skip_principals = true             # default: a name that is an agent's principal is left out
[trusted_requesters.scope]         # optional: a name's agents (main included); none: every agent
"Example Person" = ["main"]
```

Paths in the file (the persona, `from_config`'s file) are absolute, and a channel both listed and
excluded is refused. Only one key of the `from_config` file is read (an array of strings, trimmed, empties
dropped); nothing else of it reaches an error, the report or the manifest. A principal is left
out because in the reference server the principal's asks were that agent's work, approved per
text, never the owner's own.

#### What the import makes of it

Before anything is written, each agent's room (exactly one archived room with that `person`, not
Main, and the one `room_id` names: a slip that names another person's room is refused, naming
the room the archive has), its project (exactly one mapped project of that name), its tables (each with its layout's
columns) and its channels (the listed ones, then the threads table's, each once) are found in
the archive. Then, in the import's one transaction, every row is checked back as ADR 0004's
mappings are:

- **Configured rows**: the approvers, the agent (always `enabled = false`: it runs side by side
  with the old setup until the owner enables it), its lists and the trusted requesters, written
  through the agent store's own checks, read back and compared with the file.
- **The room**: mapped like an owner's room, named after the agent (counted), so its messages
  are mapped (`mine = 1` is the agent's inbound turns, its `[Slack] ...` heads kept as written)
  and the tasks made in it keep their room: they are the agent's, never no one's or the
  owner's. Tasks the source marks as asked for by the principal but made elsewhere stay where
  they were made, and are counted.
- **Facts**: after every file is read back, `context/<category>/` is renamed
  `context/agent-<id>/` in the work folder and each moved file is read back there with the
  SHA-256 it was copied with; fact pieces of recall (`source = fact`, reference
  `<category>/<slug>`) are re-tagged to match and map back through the inverse.
- **The threads layout** (`channel`, `ts`, `at`, `who`, `last`; keyed on channel and ts) becomes
  `agent_threads`, the channel's Slack team taken from the agent's channel; a channel the agent
  does not have fails the import, naming the row.
- **The approvals layout** (`id`, `task`, `kind`, `what`, `page`, `text`, `question`, `key`,
  `files` as `[{path, name, sha}]`, `files_key`, `at`, `channel`, `thread_ts`, `ts`, `ok_user`,
  `ok_channel`, `ok_ts`, `ok_said`, `ok_at`, `spent_at`, `spent_task`, and the prefixed
  `post`, `channel`, `approval`, `checked`) becomes `approval_requests` and `approval_files`
  through `ApprovalStore::import`'s own write, with `source_ref = import:<table>#<id>`, the
  file's size unknown, and the target and summary from `page` and `what` (`(not recorded)`
  when blank, counted). How a row ended, the first that holds: used (`spent_at`, with `ok_at`
  and `ok_user`; `used_by = task:<spent_task>`), else `expired` with its reason: approved and
  not used before the import; answered without a recorded approver; held by the approver's stop
  word; never answered. A decision marked as made in the approver's own app is `webhook`, any
  other `slack`. A question put to the approver stays in the archive. No row is `pending` or
  `approved`, and the import checks that none is. Each row maps back column by column, and its
  state, decision channel, files and `files_key` (the distinct hashes, sorted, comma-joined) are
  checked against its source row; the stored hash must be its payload's under the import tag.
  A row's kind is not raised by the fixed core; the rows it would give the owner today are
  counted.
- **The used layout** (`approval`, `at`, `why`) stays in the archive; the report counts its
  rows of an imported request and of none.

Not mapped: the reference server's recall pieces of the assistant's own history (they stay as
they are, as the owner's), its per-agent model, rate and file-read roots, and its log copy of the
assistant's direct messages: none of these has a place in this model.

## Consequences

- An installation with no agents rows behaves exactly as before: the default agent, the owner's
  OKs, the guard rails.
- A client-facing assistant is configuration: an agent row, its channels, its MCP servers, a
  policy per kind, and an approver. Client-specific code lives in that client's MCP server.
- Approvals are one table, reviewable in the apps, with the exact payload, who decided, how,
  and where it was used.
- New secrets, read at every use: `approval-link-key`, `agent-<id>-inbound`, an agent channel's
  bot token, `slack-<team>-signing`, an approver's webhook secret.
- The Slack DM and webhook adapters need the server reachable from outside (`EMBERHUT_PUBLIC_URL`)
  for the buttons and the webhook's answer; the signed link needs it too. Without it, approvals
  are made in the apps only.
