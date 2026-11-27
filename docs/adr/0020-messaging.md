# 0020. Messaging: Slack, Gmail, voice notes, the assistant's own WhatsApp number, and sending as the owner behind an OK

- Status: Proposed
- Date: 2026-11-26

## Context

The server reads the owner's messages from outside the app and lets the assistant act on
them:

- **Slack**, mirrored into one table from Slack's notifications on the owner's Mac and the Slack
  app's saved state (both through the Mac helper), and, in a workspace where the owner installed
  a Slack app with a user token, from Slack's Web API (a search for the owner's own messages
  every 2 minutes, each conversation read fresh when it is read, and live events over Socket
  Mode). `slack_file` fetches a message's file. `slack_send` posts one message as the owner.
- **Gmail**, read only: the server's own OAuth client, a one-time sign-in link, Gmail's history
  every 2 minutes, and live reads and searches of the whole mailbox.
- **Voice notes** in any of these, transcribed once with a speech-to-text service.
- **iMessage, SMS and WhatsApp** from the owner's Mac, read live through the Mac helper
  (`messages_chats`, `messages_read`, `messages_search`), and **`replied`**: did the owner
  already answer in a chat?
- **The assistant's own WhatsApp number**: a daemon holds the connection; the server keeps the
  allow-list (only the owner's Allow puts a chat on it), wakes the assistant only for a message
  that addresses it, and enforces the pace and the kill switch.
- **`whatsapp_send`**: one message as the owner through WhatsApp Web in the server's browser.

Sending anything as the owner is guarded: the first call sends nothing and asks the owner, on
the task, to Send it or Cancel, with the chat and the exact text; only that answer, for that
text, sends it, once. Other OKs count too (a tap on the card, a go in the chat after seeing the
text, an OK of the exact text on another card, a standing OK for a 1:1 chat); those live in the
guard rails' area. The asks, notes and answers live on tasks, which are the tasks area's.

## Decision

- **One area, its own modules.** The rules without I/O are `emberhut_core::messaging` (chat ids,
  what Slack or WhatsApp would show as formatting, the mail parsing, how a voice note reads and
  when it is retried, the WhatsApp pace rules). The tables are one migration
  (`m20261126_000012_messaging`: `slack`, `mail`, `voice_notes`, `wa_chats`, `wa_msgs`, `wa_sent`)
  with a store each in `emberhut_db::messaging`; the same migration adds `issues.last_ask` and
  `issue_events.answers` for the send guard (below), null on rows an imported archive brings in. The
  services are `services::messaging`, reached as
  `AppState::messaging()`, and one `Dependencies::messaging` field holds what they reach.
  Times are milliseconds since the epoch, as everywhere in the database.
- **Every outside service is a trait** with a real implementation and a stand-in:
  `SlackApi` (`HttpSlack`), `GoogleApi` (`HttpGoogle`), `Transcriber` (`OpenAiTranscriber`, any
  service speaking OpenAI's `/audio/transcriptions`), `MacMessages` (`HttpMacMessages`, the
  same helper `services::mac` reaches, its message routes), `WaDaemon` (`HttpWaDaemon`) and
  `WhatsAppWeb`. The HTTPS client is rustls with ring and the Mozilla roots, as the push
  services'. Tests use fakes and never reach the network.
- **Tokens are files in the secrets directory**, read at every use, so one saved later through
  a one-time secret page is used from then on: `slack-<workspace>-user`, `gmail-client-id`,
  `gmail-client-secret`, `gmail-<account>-refresh` (written by the Gmail callback, never shown),
  `openai-api-key`, and `whatsapp-token` (the bearer the WhatsApp daemon and the server send each
  other). The configuration holds addresses only: `EMBERHUT_SLACK_API`, `EMBERHUT_GMAIL_API` (one
  stand-in base for Google's three endpoints), `EMBERHUT_GMAIL_CALLBACK` (left out:
  `<public url>/gmail/callback`), `EMBERHUT_OPENAI_URL` and `EMBERHUT_TRANSCRIBE_MODEL` (shared
  with dictation, ADR 0019), `EMBERHUT_VOICE_MAX_SECONDS` (600), `EMBERHUT_WHATSAPP_URL` (the daemon; left
  out, none), `EMBERHUT_WHATSAPP_NAME` (the name a group message starts with to address the
  assistant, default `emberhut`), `EMBERHUT_WHATSAPP_OWNER` (the owner's number, so the owner is named in
  the chats) and `EMBERHUT_WHATSAPP_PHONE` (the number to link).
- **Collecting** runs in the background from `serve`: every 2 minutes Slack's workspaces are
  connected (each `slack-<workspace>-user` secret; a failure is tried again in 10 minutes) and the
  owner's own messages searched, Slack's notifications and the Slack app's saved messages are read
  from the Mac, and each Gmail account's history is read; every minute the voice notes due a try
  are started (three at a time).
- **The guard on sending as the owner is here, whole** (`services::messaging::guard`),
  for both `slack_send` and `whatsapp_send`: the waiting message per task and the last one sent
  are kept in the settings (`slackPending`, `slackSent`, `waPending`, `waSent`), and one call
  runs at a time per task (one at a time for WhatsApp Web, which has one tab). The rules:
  - The owner's answer counts only for the very ask that showed the message. Every ask on a task,
    a re-check (`same`) included, has its own id, the history line it wrote (`issues.last_ask`
    holds the latest), and every answer of the owner's records the ask it answered
    (`issue_events.answers`, the task's latest ask, read in the answer's transaction). A card's
    question is no identity: another card can carry the same words and a harmless brief.
  - Any later ask on the task replaces the waiting message: it is dropped, and the next call asks
    again. So does age: a card waits 24 hours, and a Send it counts for one hour.
  - The waiting and sent messages are each changed in one transaction
    (`SettingStore::update`), and a send first takes its waiting message off in such a step, so
    two tasks writing at once can't bring back a message that already went, and two calls can't
    both send it.
  - The same text to the same chat on a task is refused once it went out, whatever OK comes
    later; a changed text is a new ask; an OK is spent even when the channel then fails.
  - An answer to an ask that approves something to go out (`kind: approve`) takes a signed-in
    device, in report mode too (`taskDecide` answers 401 `signIn` without one). While sign-in is
    only reported, a message sent as the owner goes out only on its own card's answer or the
    standing list: a go in the chat or another card's OK may come from a request without a
    session, so they don't count then.

  The parts that belong to other areas are **hooks**
  (`services::messaging::hooks`): `Tasks` (make a task, ask on it and get the ask's id, note on
  it, the owner's answers to one ask with the task's latest ask, mark it done, a project's room)
  for the tasks area, and `SendOks` (a tap
  on the card, the go in the chat and spending it, an OK of the exact text, a standing OK, that
  a text went out, closing a card) for the guard rails. Their stand-ins are the safe ones:
  `NoTasks` cannot ask, so a server without the tasks sends nothing as the owner (the server
  passes its own tasks), and `CardOnly` counts only the card's Send it. For the guard rails' send-on-tap, the Slack and
  WhatsApp senders give the card waiting on a task (`card`) and act on the owner's tap on it
  (`answer_card`), through the same guard.
- **WhatsApp Web's browser is a hook too.** `whatsapp_send`'s guard, its chat lookup on the
  Mac (`/wa-contact`) and its log are here; typing the draft and pressing Send are
  `WhatsAppWeb`'s, which the browser area implements. Until then `NoWhatsAppWeb` answers that
  WhatsApp Web isn't available here, before anything is asked.
- **The assistant's own WhatsApp number**: the daemon reaches `POST /api/whatsapp/in` with the
  `whatsapp-token` secret as its bearer (no token saved: nobody gets in), and the app and the
  daemon's command line `GET`/`POST /api/whatsapp` with a signed-in device, in either auth mode (the
  state lists the chats and what was sent; the switch is the owner's). The assistant's `wa_switch`
  only switches replies off: what it reads (a group's messages) could ask it to turn them back on,
  so only the owner's `POST /api/whatsapp` does that. A group reply whose send the daemon refused
  (an error status) may be answered again; one with no answer to go by (a timeout, no connection)
  stays answered and is logged as unconfirmed, since the daemon queues what it sends and it may
  still go out. The one-time QR page (`GET /wa/{token}`, 10 minutes) draws the QR with the `qrcode`
  crate. The number's asks wake the assistant as `whatsapp` events in the chat's room (the `WhatsApp
  groups` project's room through the `Tasks` hook, or the main room without it). The owner's answers
  on its asks reach `WhatsAppService::decide` and `task_message`, for the tasks area to call. The
  daemon is expected to run on its own: how it is started is specific to one installation. Times in
  its context lines are UTC.
- **Words.** The owner's own messages are labelled `Owner` and the assistant's `Assistant`
  (`emberhut_core::messaging::OWNER`, `ASSISTANT`), never a person's name. The tools'
  descriptions say "the user".
- **Not in this change**: Slack's live events over Socket Mode. They need a WebSocket
  client, and without them nothing is lost but immediacy: the owner's own messages are searched
  every 2 minutes, a conversation is read fresh from Slack whenever it is read, and the Mac's
  notifications cover everyone else's. They can follow behind the same `SlackApi` trait.

## Consequences

- With nothing configured the server starts and serves as before; each source comes alive when
  its address is configured or its token saved.
- Sending to Slack or WhatsApp as the owner goes through the guard rails' send-on-tap
  (ADR 0018). `whatsapp_send` goes through `WhatsAppWeb`, whose stand-in answers that WhatsApp
  Web isn't available here: the browser's WhatsApp Web calls (ADR 0021) are not behind it yet.
  The tasks are the server's own, and they hand the assistant's WhatsApp number its answers.
- The tables keep times as milliseconds and the WhatsApp chat's project as its room's id, so an
  imported archive maps onto them one to one.
