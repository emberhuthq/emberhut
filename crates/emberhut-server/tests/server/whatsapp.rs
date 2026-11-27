//! The assistant's own WhatsApp number: the daemon's news (`POST /api/whatsapp/in`, only with
//! its token), the allow-list the owner keeps, a message that asks waking the assistant,
//! `wa_group_reply`'s rules, the kill switch (`wa_switch`, `POST /api/whatsapp`), the state
//! (`wa_groups`, `GET /api/whatsapp`), a chat's settings (`wa_chat_set`), linking (`wa_link`,
//! `GET /wa/{token}`); and `whatsapp_send`, the owner's own message through WhatsApp Web, only
//! after the owner's Send it.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure fails the test that called them"
)]

use emberhut_core::RoomId;
use serde_json::{Value, json};

use emberhut_server::services::messaging::whatsapp::DaemonError;

use crate::common::{Call, text};
use crate::messaging_fakes::Kit;

const GROUP: &str = "120363000001@g.us";

/// What the daemon tells the server, with its token.
async fn tell(kit: &Kit, event: &Value) -> crate::common::Answer {
    kit.app
        .call(
            Call::post("/api/whatsapp/in")
                .header("authorization", "Bearer daemon-token")
                .json(event),
        )
        .await
}

/// A kit whose number is in the book club group, allowed by the owner.
async fn allowed_group() -> Kit {
    let kit = Kit::new().await;
    kit.secret("whatsapp-token", "daemon-token");
    let added = tell(
        &kit,
        &json!({ "kind": "group", "chat": GROUP, "name": "Book Club", "by": "15550100777@s.whatsapp.net" }),
    )
    .await;
    assert_eq!(added.json(), json!({ "ok": true }));
    let (task, title) = kit.tasks.made()[0].clone();
    assert_eq!(title, "The assistant's WhatsApp: the group \"Book Club\"");
    assert_eq!(
        kit.tasks.asks(task)[0].question,
        "The assistant was added to \"Book Club\" by +15550100777: allow?"
    );
    assert!(
        kit.app
            .state
            .messaging()
            .whatsapp()
            .decide(task, "Allow")
            .await
            .unwrap()
    );
    assert_eq!(kit.tasks.done(), [task]);
    kit
}

fn message(id: &str, text: &str, mentioned: bool) -> Value {
    json!({
        "kind": "message", "group": true, "chat": GROUP, "name": "Book Club", "id": id,
        "sender": "15550100200@s.whatsapp.net", "who": "Ann", "text": text, "mentioned": mentioned
    })
}

#[tokio::test]
pub(crate) async fn the_daemon_needs_its_token_and_unknown_news_is_refused() {
    let kit = Kit::new().await;
    let none = kit
        .app
        .call(Call::post("/api/whatsapp/in").json(&json!({ "kind": "status" })))
        .await;
    assert_eq!(none.status, 401, "no token set up: nobody gets in");
    kit.secret("whatsapp-token", "daemon-token");
    let wrong = kit
        .app
        .call(
            Call::post("/api/whatsapp/in")
                .header("authorization", "Bearer guess")
                .json(&json!({ "kind": "status" })),
        )
        .await;
    assert_eq!(wrong.status, 401);
    let odd = tell(&kit, &json!({ "kind": "weather" })).await;
    assert_eq!(odd.status, 400);
    assert_eq!(odd.json()["error"], "unknown kind weather");
    let status = tell(
        &kit,
        &json!({ "kind": "status", "state": "open", "number": "15550100999" }),
    )
    .await;
    assert_eq!(status.json(), json!({ "ok": true }));
}

#[tokio::test]
pub(crate) async fn only_an_allowed_chat_is_kept_and_only_an_ask_wakes_the_assistant() {
    let kit = Kit::new().await;
    kit.secret("whatsapp-token", "daemon-token");
    let stranger = tell(
        &kit,
        &json!({ "kind": "message", "chat": "15550100300@s.whatsapp.net", "text": "hi", "id": "x1" }),
    )
    .await;
    assert_eq!(
        stranger.json(),
        json!({ "dropped": "not on the allow-list" })
    );
    let unasked = tell(&kit, &message("g0", "hello all", false)).await;
    assert_eq!(
        unasked.json(),
        json!({ "dropped": "not allowed yet" }),
        "a group nobody allowed only gets the owner asked"
    );
    assert_eq!(kit.tasks.made().len(), 1);

    let task = kit.tasks.made()[0].0;
    kit.app
        .state
        .messaging()
        .whatsapp()
        .decide(task, "Allow")
        .await
        .unwrap();
    let chatter = tell(&kit, &message("g1", "anyone read chapter 3?", false)).await;
    assert_eq!(chatter.json(), json!({ "kept": true }));
    let asked = tell(&kit, &message("g2", "Nova, when do we meet?", false)).await;
    assert_eq!(
        asked.json(),
        json!({ "kept": true, "asked": true }),
        "it starts with the name"
    );
    let mention = tell(&kit, &message("g3", "what do you think?", true)).await;
    assert_eq!(mention.json(), json!({ "kept": true, "asked": true }));

    let events = kit.app.state.messaging().whatsapp().state().await.unwrap();
    assert_eq!(events.chats[0].status, "allowed");
    let db = emberhut_db::Database::connect(&kit.app.url).await.unwrap();
    let waiting = db.events().waiting(RoomId::MAIN, 100).await.unwrap();
    let wake = waiting.iter().find(|e| e.kind == "whatsapp").unwrap();
    assert!(wake.text.starts_with("WhatsApp, the group \"Book Club\": Ann talked to you (message g2): «Nova, when do we meet?»"), "{}", wake.text);
    assert!(
        wake.text.contains("Ann: anyone read chapter 3?"),
        "the context: {}",
        wake.text
    );
    assert!(wake.text.contains(&format!(
        "wa_group_reply (group: \"{GROUP}\", quoted: \"g2\")"
    )));
    assert_eq!(waiting.iter().filter(|e| e.kind == "whatsapp").count(), 2);
}

#[tokio::test]
pub(crate) async fn wa_group_reply_answers_once_per_ask_at_the_pace_rules() {
    let kit = allowed_group().await;
    tell(&kit, &message("g1", "Nova, is it Friday?", false)).await;
    kit.daemon.answers("/send", Ok(json!({ "id": "out1" })));

    let unprompted = kit
        .app
        .call_tool(
            "wa_group_reply",
            json!({ "group": "Other group", "text": "hi" }),
        )
        .await;
    assert!(text(&unprompted).contains("isn't on the assistant's WhatsApp allow-list"));
    let markdown = kit
        .app
        .call_tool(
            "wa_group_reply",
            json!({ "group": GROUP, "text": "**Yes**" }),
        )
        .await;
    assert!(text(&markdown).starts_with("Not sent: markdown"));
    let link = kit
        .app
        .call_tool(
            "wa_group_reply",
            json!({ "group": GROUP, "text": "see example.com" }),
        )
        .await;
    assert!(text(&link).starts_with("Not sent: no links"));

    let sent = kit
        .app
        .call_tool(
            "wa_group_reply",
            json!({ "group": "book club", "text": "Yes, Friday at 7." }),
        )
        .await;
    assert!(
        text(&sent).starts_with("Sent in \"Book Club\" at "),
        "{}",
        text(&sent)
    );
    let body = kit.daemon.calls("/send")[0].clone().unwrap();
    assert_eq!(body["chat"], GROUP);
    assert_eq!(body["text"], "Yes, Friday at 7.");
    assert_eq!(body["quoted"]["id"], "g1");

    let twice = kit
        .app
        .call_tool(
            "wa_group_reply",
            json!({ "group": GROUP, "text": "Again", "quoted": "g1" }),
        )
        .await;
    assert!(text(&twice).contains("already answered that message"));
    tell(&kit, &message("g2", "nova, and the time?", false)).await;
    let soon = kit
        .app
        .call_tool("wa_group_reply", json!({ "group": GROUP, "text": "At 7." }))
        .await;
    assert!(
        text(&soon).contains("at most one reply per 10 s"),
        "{}",
        text(&soon)
    );
    kit.later(11);
    kit.daemon.answers(
        "/send",
        Err(DaemonError::Refused("the number is offline".to_owned())),
    );
    let failed = kit
        .app
        .call_tool("wa_group_reply", json!({ "group": GROUP, "text": "At 7." }))
        .await;
    assert_eq!(text(&failed), "Not sent: the number is offline.");
    kit.daemon.answers("/send", Ok(json!({ "id": "out2" })));
    let retried = kit
        .app
        .call_tool("wa_group_reply", json!({ "group": GROUP, "text": "At 7." }))
        .await;
    assert!(
        text(&retried).starts_with("Sent in"),
        "a failed send leaves the ask open: {}",
        text(&retried)
    );

    let groups = kit.app.call_tool("wa_groups", json!({})).await;
    let groups: Value = serde_json::from_str(text(&groups)).unwrap();
    assert_eq!(groups["sent"][0]["text"], "At 7.");
    assert_eq!(groups["chats"][0]["name"], "Book Club");
    let session = kit.app.sign_in().await;
    let state = kit
        .app
        .call(Call::get("/api/whatsapp").token(&session))
        .await;
    assert_eq!(state.status, 200);
    assert_eq!(state.json()["sent"].as_array().unwrap().len(), 2);
}

#[tokio::test]
pub(crate) async fn the_kill_switch_stops_every_reply() {
    let kit = allowed_group().await;
    tell(&kit, &message("g1", "nova?", false)).await;
    kit.daemon.answers("/stop", Ok(json!({ "dropped": 2 })));
    let off = kit.app.call_tool("wa_switch", json!({ "on": false })).await;
    assert_eq!(
        text(&off),
        "The assistant's WhatsApp replies are off: nothing goes out until it's turned back on; 2 queued messages dropped."
    );
    let db = emberhut_db::Database::connect(&kit.app.url).await.unwrap();
    assert_eq!(
        db.events()
            .waiting(RoomId::MAIN, 100)
            .await
            .unwrap()
            .iter()
            .filter(|e| e.kind == "whatsapp")
            .count(),
        0
    );
    let reply = kit
        .app
        .call_tool("wa_group_reply", json!({ "group": GROUP, "text": "hi" }))
        .await;
    assert!(text(&reply).contains("switched off"));
    let off_ask = tell(&kit, &message("g2", "nova!", false)).await;
    assert_eq!(off_ask.json(), json!({ "kept": true, "off": true }));

    let session = kit.app.sign_in().await;
    let on = kit
        .app
        .call(
            Call::post("/api/whatsapp")
                .token(&session)
                .json(&json!({ "kind": "on" })),
        )
        .await;
    assert_eq!(
        on.json(),
        json!({ "ok": true, "text": "The assistant's WhatsApp replies are on." })
    );
    let unsigned = kit
        .app
        .call(Call::post("/api/whatsapp").json(&json!({ "kind": "off" })))
        .await;
    assert_eq!(unsigned.status, 401);
    let unseen = kit.app.call(Call::get("/api/whatsapp")).await;
    assert_eq!(unseen.status, 401);
    let odd = kit
        .app
        .call(
            Call::post("/api/whatsapp")
                .token(&session)
                .json(&json!({ "kind": 5 })),
        )
        .await;
    assert_eq!(odd.status, 400);
}

#[tokio::test]
pub(crate) async fn wa_chat_set_allows_only_through_the_owner_and_keeps_topics() {
    let kit = allowed_group().await;
    let topics = kit
        .app
        .call_tool(
            "wa_chat_set",
            json!({ "chat": GROUP, "topics": "books only", "project": "Garden" }),
        )
        .await;
    assert_eq!(
        text(&topics),
        "\"Book Club\": its asks now go to the Garden room; allowed there: books only."
    );
    let cleared = kit
        .app
        .call_tool("wa_chat_set", json!({ "chat": GROUP, "topics": null }))
        .await;
    assert_eq!(text(&cleared), "\"Book Club\": no private topics there.");
    let nowhere = kit
        .app
        .call_tool(
            "wa_chat_set",
            json!({ "chat": GROUP, "project": "Nowhere" }),
        )
        .await;
    assert_eq!(text(&nowhere), "No project \"Nowhere\".");

    let dm = kit
        .app
        .call_tool(
            "wa_chat_set",
            json!({ "chat": "+1 555 010 0400", "allow": true, "name": "Cy" }),
        )
        .await;
    assert!(
        text(&dm).starts_with("DMs with Cy: the owner was asked to allow it (task "),
        "{}",
        text(&dm)
    );
    let state = kit.app.state.messaging().whatsapp().state().await.unwrap();
    let cy = state.chats.iter().find(|c| c.name == "Cy").unwrap();
    assert_eq!(cy.status, "pending", "never allowed by itself");
    let removed = kit
        .app
        .call_tool(
            "wa_chat_set",
            json!({ "chat": "book club", "remove": true }),
        )
        .await;
    assert_eq!(
        text(&removed),
        "\"Book Club\": off the allow-list (the assistant stays quiet there)."
    );
}

#[tokio::test]
pub(crate) async fn linking_gives_a_pairing_code_or_a_one_time_qr_page() {
    let kit = Kit::new().await;
    let need = kit.app.call_tool("wa_link", json!({})).await;
    assert_eq!(
        text(&need),
        "Need the assistant's number first (with its country code)."
    );
    kit.daemon
        .answers("/status", Ok(json!({ "state": "linking" })));
    kit.daemon
        .answers("/link", Ok(json!({ "code": "ABCDEFGH" })));
    let code = kit
        .app
        .call_tool(
            "wa_link",
            json!({ "phone": "+1 555 010 0999", "task_id": 5 }),
        )
        .await;
    assert!(
        text(&code).starts_with("Pairing code for +15550100999: ABCD-EFGH"),
        "{}",
        text(&code)
    );
    assert!(kit.tasks.asks(5)[0].question.contains("ABCD-EFGH"));
    assert_eq!(
        kit.daemon.calls("/link")[0],
        Some(json!({ "phone": "+15550100999" }))
    );

    kit.daemon.answers("/link", Ok(json!({})));
    let session = kit.app.sign_in().await;
    let qr = kit
        .app
        .call(
            Call::post("/api/whatsapp")
                .token(&session)
                .json(&json!({ "kind": "qr" })),
        )
        .await;
    let url = qr.json()["qr"].as_str().unwrap().to_owned();
    assert!(url.starts_with("https://box.example/wa/"), "{url}");
    let path = url.trim_start_matches("https://box.example");
    kit.daemon.answers(
        "/status",
        Ok(json!({ "state": "linking", "linking": { "qr": "2@abc,def" } })),
    );
    let page = kit.app.call(Call::get(path)).await;
    assert_eq!(page.status, 200);
    assert!(page.text.contains("<svg"), "{}", page.text);
    assert_eq!(page.header("cache-control"), "no-store");
    kit.daemon
        .answers("/status", Ok(json!({ "state": "open" })));
    let linked = kit.app.call(Call::get(path)).await;
    assert!(linked.text.contains("Linked."));
    let used = kit.app.call(Call::get(path)).await;
    assert_eq!(used.status, 404);

    kit.secret("whatsapp-token", "daemon-token");
    tell(&kit, &json!({ "kind": "status", "state": "linking" })).await;
    tell(
        &kit,
        &json!({ "kind": "status", "state": "open", "number": "15550100999" }),
    )
    .await;
    let state = kit
        .app
        .call(Call::get("/api/whatsapp").token(&session))
        .await;
    assert_eq!(state.json()["state"], "open");
    assert_eq!(state.json()["number"], "15550100999");
    tell(
        &kit,
        &json!({ "kind": "status", "state": "logged_out", "error": "logged out on the phone" }),
    )
    .await;
    let db = emberhut_db::Database::connect(&kit.app.url).await.unwrap();
    let urgent = db.events().waiting(RoomId::MAIN, 100).await.unwrap();
    assert!(
        urgent
            .iter()
            .any(|e| e.kind == "urgent" && e.text.contains("logged out on the phone"))
    );
}

#[tokio::test]
pub(crate) async fn whatsapp_send_types_the_draft_and_sends_only_after_the_owner_s_send_it() {
    let kit = Kit::new().await;
    kit.mac.answers(
        "/wa-contact",
        &json!({ "chat": "wa:12", "name": "Bo Park", "kind": "person", "phone": "15550100500", "me": false }),
    );
    let args = json!({ "chat": "wa:12", "text": "On my way", "task_id": 3 });
    let first = kit.app.call_tool("whatsapp_send", args.clone()).await;
    assert!(
        text(&first).starts_with("Typed in WhatsApp Web, not sent."),
        "{}",
        text(&first)
    );
    assert_eq!(kit.web.drafts()[0].1, "On my way");
    assert_eq!(kit.web.sent().len(), 0);
    assert_eq!(
        kit.tasks.asks(3)[0].question,
        "Send this WhatsApp message to Bo Park?"
    );
    let waiting = kit.app.call_tool("whatsapp_send", args.clone()).await;
    assert!(text(&waiting).starts_with("Still waiting"));

    kit.later(60);
    kit.tasks.choose(3, "Send it", kit.now());
    let sent = kit.app.call_tool("whatsapp_send", args.clone()).await;
    assert_eq!(
        text(&sent),
        "Sent to Bo Park on WhatsApp: delivered (✓✓). Logged on task 3."
    );
    assert_eq!(kit.web.sent(), ["On my way"]);
    let again = kit.app.call_tool("whatsapp_send", args).await;
    assert!(text(&again).starts_with("Already sent to Bo Park"));
    assert_eq!(kit.web.sent().len(), 1);

    let missing = kit
        .app
        .call_tool("whatsapp_send", json!({ "chat": "wa:12", "text": "x" }))
        .await;
    assert!(text(&missing).contains("needs task_id"));
    kit.web
        .refuses(emberhut_server::services::messaging::whatsapp_web::WebSendError::NotAvailable);
    let no_browser = kit
        .app
        .call_tool(
            "whatsapp_send",
            json!({ "chat": "wa:12", "text": "Hi", "task_id": 4 }),
        )
        .await;
    assert!(text(&no_browser).starts_with("WhatsApp Web isn't available on this server"));
    assert_eq!(kit.tasks.asks(4).len(), 0, "nothing asked without a draft");
}

#[tokio::test]
async fn a_whatsapp_draft_kept_as_a_card_is_sent_on_the_owner_s_tap() {
    use emberhut_server::services::messaging::guard::{CardAnswer, Draft};
    let kit = Kit::new().await;
    kit.mac.answers(
        "/wa-contact",
        &json!({ "chat": "wa:12", "name": "Bo Park", "kind": "person", "phone": "15550100500", "me": false }),
    );
    let send = kit.app.state.messaging().whatsapp_send();
    let draft = Draft {
        chat: "wa:12".to_owned(),
        text: "Running ten minutes late".to_owned(),
        question: "Tell Bo you're late?".to_owned(),
        ask: kit.tasks.ask_card(30, "Tell Bo you're late?"),
        send: "Send it".to_owned(),
        room: Some(RoomId::MAIN),
    };
    let prepared = send.prepare(30, &draft).await.unwrap();
    assert_eq!(prepared.line, "Bo Park on WhatsApp (wa:12)");
    let card = send.card(30).await.unwrap().unwrap();
    assert_eq!(card.tool, "whatsapp_send");
    kit.oks.tap(30, card.at);
    let answer = send.answer_card(30, "Send it", &card).await.unwrap();
    assert!(matches!(answer, CardAnswer::Sent(_)), "{answer:?}");
    assert_eq!(kit.web.sent(), ["Running ten minutes late"]);

    let draft = Draft {
        ask: kit.tasks.ask_card(31, "Tell Bo you're late?"),
        ..draft
    };
    send.prepare(31, &draft).await.unwrap();
    let card = send.card(31).await.unwrap().unwrap();
    assert_eq!(
        send.answer_card(31, "Cancel", &card).await.unwrap(),
        CardAnswer::Cancelled
    );
    assert_eq!(send.card(31).await.unwrap(), None);
}

#[tokio::test]
async fn a_press_of_send_that_fails_is_never_a_send() {
    use emberhut_server::services::messaging::whatsapp_web::WebSendError;
    let kit = Kit::new().await;
    kit.mac.answers(
        "/wa-contact",
        &json!({ "chat": "wa:12", "name": "Bo Park", "kind": "person", "phone": "15550100500", "me": false }),
    );
    let errors = [
        WebSendError::NotLoaded,
        WebSendError::NotOne("Two chats are called Bo Park.".to_owned()),
        WebSendError::NotOpened("The chat didn't open.".to_owned()),
        WebSendError::Changed,
        WebSendError::NotLinked("Scan the code.".to_owned()),
        WebSendError::NotAvailable,
        WebSendError::Failed("the page went away".to_owned()),
    ];
    for (task, error) in (40..).zip(errors) {
        let args = json!({ "chat": "wa:12", "text": "On my way", "task_id": task });
        kit.app.call_tool("whatsapp_send", args.clone()).await;
        kit.later(60);
        kit.tasks.choose(task, "Send it", kit.now());
        kit.web.fails_to_send(Some(error.clone()));
        let failed = text(&kit.app.call_tool("whatsapp_send", args.clone()).await).to_owned();
        assert_eq!(
            failed,
            format!(
                "Not sent: {error}. The owner's OK is spent; if they still want it, ask again."
            )
        );
        assert!(!failed.contains("not confirmed"), "{failed}");
        assert!(kit.web.sent().is_empty(), "{error:?}");
        assert!(
            kit.tasks.notes(task).iter().all(|n| !n.starts_with("Sent")),
            "{:?}",
            kit.tasks.notes(task)
        );
        assert!(kit.oks.sent().contains(&(
            task,
            "On my way".to_owned(),
            "WhatsApp (failed)".to_owned()
        )));

        kit.web.fails_to_send(None);
        let again = text(&kit.app.call_tool("whatsapp_send", args).await).to_owned();
        assert!(
            again.starts_with("Typed in WhatsApp Web, not sent."),
            "no send was recorded, so the same text is a new ask, not a resend: {again}"
        );
        assert_eq!(kit.tasks.asks(task).len(), 2);
    }
}

#[tokio::test]
async fn the_owner_s_allow_on_the_server_s_own_task_card_lets_the_group_in() {
    use emberhut_api::paths;
    use emberhut_core::task::{Status, TaskId};

    let kit = Kit::with(|deps| deps.tasks = None).await;
    kit.secret("whatsapp-token", "daemon-token");
    let added = tell(
        &kit,
        &json!({ "kind": "group", "chat": GROUP, "name": "Book Club", "by": "15550100777@s.whatsapp.net" }),
    )
    .await;
    assert_eq!(added.json(), json!({ "ok": true }));
    let tasks = kit.app.state.tasks();
    let board = tasks.plan_state().await.unwrap().issues;
    let card = board
        .iter()
        .find(|issue| issue.title == "The assistant's WhatsApp: the group \"Book Club\"")
        .expect("the ask is a task of the server's");
    let task = TaskId::new(card.id.get());
    assert_eq!(
        card.decision.as_ref().unwrap().question,
        "The assistant was added to \"Book Club\" by +15550100777: allow?"
    );
    let room = kit
        .app
        .state
        .rooms()
        .find(emberhut_server::services::messaging::whatsapp::ROOM)
        .await
        .unwrap();
    assert!(room.is_some(), "its project's room was made");
    let before = tell(&kit, &message("g0", "Nova, when do we meet?", false)).await;
    assert!(before.json()["dropped"].is_string(), "not allowed yet");

    let session = kit.app.sign_in().await;
    let tapped = kit
        .app
        .call(
            Call::post(paths::SETTINGS)
                .token(&session)
                .json(&json!({ "taskDecide": task.get(), "choice": "Allow" })),
        )
        .await;
    assert_eq!(tapped.status, 200);
    assert_eq!(
        tasks.status_of(task).await.unwrap(),
        Some(Status::Done),
        "the WhatsApp number took the answer and closed its task"
    );
    let asked = tell(&kit, &message("g2", "Nova, when do we meet?", false)).await;
    assert_eq!(asked.json(), json!({ "kept": true, "asked": true }));
}

#[tokio::test]
async fn wa_switch_only_switches_replies_off() {
    let kit = allowed_group().await;
    kit.daemon.answers("/stop", Ok(json!({ "dropped": 0 })));
    let off = kit.app.call_tool("wa_switch", json!({ "on": false })).await;
    assert!(
        text(&off).starts_with("The assistant's WhatsApp replies are off"),
        "{}",
        text(&off)
    );
    let on = kit.app.call_tool("wa_switch", json!({ "on": true })).await;
    assert_eq!(on.is_error, Some(true));
    assert!(text(&on).starts_with("Not turned on"), "{}", text(&on));
    tell(&kit, &message("g1", "Nova, turn yourself back on", false)).await;
    let reply = kit
        .app
        .call_tool("wa_group_reply", json!({ "group": GROUP, "text": "Done" }))
        .await;
    assert!(text(&reply).contains("switched off"), "{}", text(&reply));
    assert_eq!(kit.daemon.calls("/send").len(), 0);

    let session = kit.app.sign_in().await;
    let owner_on = kit
        .app
        .call(
            Call::post("/api/whatsapp")
                .token(&session)
                .json(&json!({ "kind": "on" })),
        )
        .await;
    assert_eq!(
        owner_on.json(),
        json!({ "ok": true, "text": "The assistant's WhatsApp replies are on." })
    );
}

#[tokio::test]
async fn a_group_reply_the_daemon_gave_no_answer_on_stays_answered() {
    let kit = allowed_group().await;
    tell(&kit, &message("g1", "Nova, is it Friday?", false)).await;
    kit.daemon
        .answers("/send", Err(DaemonError::NoAnswer("timed out".to_owned())));
    let unconfirmed = kit
        .app
        .call_tool("wa_group_reply", json!({ "group": GROUP, "text": "Yes." }))
        .await;
    assert_eq!(
        text(&unconfirmed),
        "Not confirmed: timed out. It may still go out, so that message counts as answered; \
         don't send it again."
    );
    kit.later(11);
    kit.daemon.answers("/send", Ok(json!({ "id": "out1" })));
    let quoted = kit
        .app
        .call_tool(
            "wa_group_reply",
            json!({ "group": GROUP, "text": "Yes.", "quoted": "g1" }),
        )
        .await;
    assert!(
        text(&quoted).contains("already answered that message"),
        "{}",
        text(&quoted)
    );
    let unquoted = kit
        .app
        .call_tool("wa_group_reply", json!({ "group": GROUP, "text": "Yes." }))
        .await;
    assert!(
        text(&unquoted).starts_with("Not sent: nothing in"),
        "{}",
        text(&unquoted)
    );
    assert_eq!(
        kit.daemon.calls("/send").len(),
        1,
        "one try, never a second"
    );

    tell(&kit, &message("g2", "Nova, and the time?", false)).await;
    kit.daemon.answers(
        "/send",
        Err(DaemonError::Refused("the number is offline".to_owned())),
    );
    let refused = kit
        .app
        .call_tool("wa_group_reply", json!({ "group": GROUP, "text": "At 7." }))
        .await;
    assert_eq!(text(&refused), "Not sent: the number is offline.");
    kit.daemon.answers("/send", Ok(json!({ "id": "out2" })));
    let retried = kit
        .app
        .call_tool("wa_group_reply", json!({ "group": GROUP, "text": "At 7." }))
        .await;
    assert!(
        text(&retried).starts_with("Sent in"),
        "a send the daemon refused leaves the ask open: {}",
        text(&retried)
    );
}

#[tokio::test]
async fn a_chat_bound_to_a_room_an_agent_was_given_still_wakes_only_the_owners_assistant() {
    let kit = allowed_group().await;
    let db = emberhut_db::Database::connect(&kit.app.url).await.unwrap();
    let now = kit.app.clock_now();
    let room = db.rooms().create("Garden", now).await.unwrap();
    db.whatsapp()
        .change(
            GROUP,
            emberhut_db::messaging::ChatChange {
                room: Some(Some(room.id)),
                ..emberhut_db::messaging::ChatChange::default()
            },
            now,
        )
        .await
        .unwrap();
    // The room is given to a configured agent after the chat was bound to it.
    db.agents()
        .create(
            &emberhut_core::agents::Agent {
                id: emberhut_core::agents::AgentId::parse("client-a").unwrap(),
                name: "Example helper".to_owned(),
                persona: None,
                principal: "Example Ltd".to_owned(),
                room: room.id,
                project: None,
                spend_cap: None,
                enabled: true,
                worker_shell: false,
            },
            now,
        )
        .await
        .unwrap();
    let asked = tell(&kit, &message("g9", "Nova, when do we meet?", false)).await;
    assert_eq!(asked.json(), json!({ "kept": true, "asked": true }));
    let theirs = db.events().waiting(room.id, 100).await.unwrap();
    assert!(
        theirs.is_empty(),
        "nothing of the owner's chats: {theirs:?}"
    );
    let main = db.events().waiting(RoomId::MAIN, 100).await.unwrap();
    assert!(
        main.iter()
            .any(|e| e.kind == "whatsapp" && e.text.contains("message g9")),
        "{main:?}"
    );
}
