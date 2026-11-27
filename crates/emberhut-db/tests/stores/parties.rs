//! The party store: a party and its talk, its status and caps, what it used, and deleting it.
#![expect(clippy::unwrap_used, reason = "tests")]

use emberhut_core::party::{
    MessageKind, NewParty, NewPartyMessage, PartyStatus, PersonaDraft, plan,
};

use crate::common::{at, database};

fn a_plan() -> emberhut_core::party::PartyPlan {
    plan(&NewParty {
        topic: "Should cities ban cars?".to_owned(),
        personas: ["Nora", "Victor"]
            .into_iter()
            .map(|name| PersonaDraft {
                name: name.to_owned(),
                ..PersonaDraft::default()
            })
            .collect(),
        ..NewParty::default()
    })
    .unwrap()
}

#[tokio::test]
async fn a_party_keeps_its_cast_its_talk_and_its_counts() {
    let (_dir, db) = database().await;
    let parties = db.parties();
    let opening = NewPartyMessage::host(MessageKind::Say, "Should cities ban cars?");
    let party = parties
        .create(&a_plan(), PartyStatus::Running, &opening, at(0))
        .await
        .unwrap();
    assert_eq!(party.personas.len(), 2);
    assert_eq!(party.personas[1].name, "Victor");
    assert_eq!(party.status, PartyStatus::Running);

    let turn = NewPartyMessage {
        who: "p1".to_owned(),
        name: "Nora".to_owned(),
        kind: MessageKind::Turn,
        text: "Yes,\n  gladly.".to_owned(),
        model: Some("claude-opus-5-5".to_owned()),
        tokens: 120,
        cost: 0.01,
        cut: false,
    };
    parties.add_message(party.id, &turn, at(1)).await.unwrap();
    parties.add_use(party.id, true, 120, 0.01).await.unwrap();
    parties
        .add_message(
            party.id,
            &NewPartyMessage::host(MessageKind::Steer, "argue"),
            at(2),
        )
        .await
        .unwrap();
    let talk = parties.messages(party.id).await.unwrap();
    assert_eq!(talk.len(), 3);
    assert_eq!(talk[1].tokens, 120);
    let got = parties.get(party.id).await.unwrap().unwrap();
    assert_eq!((got.turns, got.tokens), (1, 120));
    assert_eq!(got.updated, at(2));

    let list = parties.list(200).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].1.as_deref(), Some("Nora: Yes, gladly."));

    assert!(parties.set_caps(party.id, Some(30), None).await.unwrap());
    assert!(
        parties
            .set_status(party.id, PartyStatus::Paused, Some("waits"), at(3))
            .await
            .unwrap()
    );
    let got = parties.get(party.id).await.unwrap().unwrap();
    assert_eq!(got.max_turns, 30);
    assert_eq!(got.why.as_deref(), Some("waits"));
    assert_eq!(parties.recent_casts(10).await.unwrap()[0].len(), 2);
}

#[tokio::test]
async fn a_restart_pauses_running_parties_and_delete_removes_the_talk() {
    let (_dir, db) = database().await;
    let parties = db.parties();
    let opening = NewPartyMessage::host(MessageKind::Say, "t");
    let running = parties
        .create(&a_plan(), PartyStatus::Running, &opening, at(0))
        .await
        .unwrap();
    let paused = parties
        .create(&a_plan(), PartyStatus::Paused, &opening, at(0))
        .await
        .unwrap();
    assert_eq!(parties.pause_running("restarted", at(1)).await.unwrap(), 1);
    let got = parties.get(running.id).await.unwrap().unwrap();
    assert_eq!(got.status, PartyStatus::Paused);
    assert_eq!(got.why.as_deref(), Some("restarted"));
    assert!(parties.delete(paused.id).await.unwrap());
    assert!(parties.get(paused.id).await.unwrap().is_none());
    assert_eq!(parties.messages(paused.id).await.unwrap(), Vec::new());
    assert!(!parties.delete(paused.id).await.unwrap());
    // The party's lines are gone from party_messages with it; the other party's stay.
    let left = party_messages_of(&db, paused.id.get()).await;
    assert_eq!(left, 0);
    assert_eq!(party_messages_of(&db, running.id.get()).await, 1);
}

/// How many rows `party_messages` holds for party `id`, read straight from the table.
async fn party_messages_of(db: &emberhut_db::Database, id: i64) -> i64 {
    use sea_orm::ConnectionTrait as _;
    let url = format!("sqlite://{}?mode=ro", db.path().display());
    let conn = sea_orm::Database::connect(url).await.unwrap();
    let row = conn
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS n FROM party_messages WHERE party = ?",
            [id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "n").unwrap()
}
