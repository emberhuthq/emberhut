//! The ask guard: an ask the owner's words already cover comes back "covered, do it" with
//! their own words; what always asks reaches them without the judge; a made-up quote, no
//! judge or a judge that fails lets the ask through.

use std::sync::Arc;

use emberhut_core::RoomId;
use emberhut_core::task::Who;
use emberhut_server::services::guard::ask_guard::TaskAsk;
use emberhut_server::services::guard::{GitRemotes, GuardConfig, NoJudge};
use serde_json::json;

use crate::guard_common::{FakeJudge, guard_app, say, task};

fn ask(action: &str, question: &str, options: &[&str]) -> TaskAsk {
    TaskAsk {
        action: Some(action.to_owned()),
        question: question.to_owned(),
        options: options.iter().map(|o| (*o).to_owned()).collect(),
        ..TaskAsk::default()
    }
}

fn verdict(covered: bool, quote: &str, tag: &str) -> String {
    json!({ "reserved": null, "covered": covered, "answer": null, "quote": quote, "tag": tag, "why": "because" })
        .to_string()
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one ask after another, as the log reads them"
)]
async fn an_ask_the_owners_words_cover_is_sent_back_with_them() {
    let judge = Arc::new(FakeJudge::default());
    let app = guard_app(judge.clone(), Arc::new(GitRemotes), GuardConfig::default()).await;
    let guard = app.state.guard();
    let request = say(
        &app,
        RoomId::MAIN,
        true,
        "run the ads on the store, do it end to end",
    )
    .await;
    let id = task(&app, "the store ads", Some(request)).await;

    *judge.answer.lock().unwrap() = Some(verdict(
        true,
        "do it end to end",
        &format!("m{}", request.get()),
    ));
    let back = guard
        .task_ask(
            id,
            Who::Worker,
            &ask(
                "Go ahead on the bids",
                "Set the bid per tap?",
                &["Go ahead", "Stop"],
            ),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        back.starts_with("Not asked: covered, do it. The owner's own words already OK this: «do it end to end» (their request for "),
        "{back}"
    );
    let prompt = judge.prompts.lock().unwrap().last().cloned().unwrap();
    assert!(prompt.contains("Set the bid per tap?"), "{prompt}");
    assert!(
        prompt.contains(&format!(
            "[m{}] The owner's request for {}",
            request.get(),
            id.key()
        )),
        "{prompt}"
    );
    let history = app.state.tasks().history_of(id).await.unwrap();
    assert!(history.iter().any(|h| {
        h.text
            .starts_with("Ask held by the ask guard: the owner's words cover it")
    }));

    *judge.answer.lock().unwrap() = Some(verdict(true, "they said yes to everything", "m1"));
    assert_eq!(
        guard
            .task_ask(id, Who::Worker, &ask("Run them", "Run the ads?", &["Yes"]))
            .await
            .unwrap(),
        None,
        "a quote that isn't theirs"
    );
    *judge.answer.lock().unwrap() = Some("I think it's covered".to_owned());
    assert_eq!(
        guard
            .task_ask(id, Who::Worker, &ask("Run", "Run?", &["Yes"]))
            .await
            .unwrap(),
        None
    );

    let asked = judge.prompts.lock().unwrap().len();
    let pay = ask(
        "Pay for the ads",
        "Add the card and pay $50?",
        &["Yes", "No"],
    );
    assert_eq!(guard.task_ask(id, Who::Worker, &pay).await.unwrap(), None);
    let message = TaskAsk {
        draft: Some("hi, the ads are live".to_owned()),
        chat: Some("wa:12".to_owned()),
        ..ask("Send Sam the note", "Send it?", &["Send it"])
    };
    assert_eq!(
        guard.task_ask(id, Who::Worker, &message).await.unwrap(),
        None
    );
    assert_eq!(
        judge.prompts.lock().unwrap().len(),
        asked,
        "the judge isn't asked"
    );

    let overridden = TaskAsk {
        not_covered: Some("the request doesn't pick a video".to_owned()),
        ..ask("Pick the video", "Which video?", &["A", "B"])
    };
    assert_eq!(
        guard.task_ask(id, Who::Worker, &overridden).await.unwrap(),
        None
    );
    let log = guard.ask_log().await.unwrap();
    let outcomes: Vec<&str> = log.iter().map(|e| e.outcome.as_str()).collect();
    assert_eq!(
        outcomes,
        [
            "covered",
            "not_covered",
            "no_verdict",
            "reserved",
            "reserved",
            "overridden"
        ]
    );
}

#[tokio::test]
async fn with_no_judge_every_ask_reaches_the_owner() {
    let app = guard_app(
        Arc::new(NoJudge),
        Arc::new(GitRemotes),
        GuardConfig::default(),
    )
    .await;
    let request = say(
        &app,
        RoomId::MAIN,
        true,
        "fix the importer, merge it and deploy",
    )
    .await;
    let id = task(&app, "the importer", Some(request)).await;
    let guard = app.state.guard();
    assert_eq!(
        guard
            .task_ask(id, Who::Worker, &ask("OK the build", "Run it?", &["Yes"]))
            .await
            .unwrap(),
        None
    );
    let merge = guard
        .task_ask(
            id,
            Who::Worker,
            &ask("OK the merge", "Merge PR #12 to main?", &["Merge", "Wait"]),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        merge.starts_with(
            "Not asked: covered, do it. Its push, merge and deploy are OK'd by their request for "
        ),
        "the outward rule needs no judge: {merge}"
    );
}
