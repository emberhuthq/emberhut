//! Settings, the activity log and events against a real SQLite database.

use emberhut_core::RoomId;
use emberhut_core::event::NewEvent;
use emberhut_db::{DbError, SettingKey};
use serde::{Deserialize, Serialize};

use crate::common::{at, database};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Shown {
    hide_goals: bool,
}

const SHOWN: SettingKey<Shown> = SettingKey::new("shown");

#[tokio::test]
async fn a_setting_is_kept_as_its_type_until_it_changes() {
    let (_dir, db) = database().await;
    let settings = db.settings();
    assert_eq!(settings.get(&SHOWN).await.unwrap(), None);
    settings
        .set(&SHOWN, &Shown { hide_goals: true })
        .await
        .unwrap();
    assert_eq!(
        settings.get(&SHOWN).await.unwrap(),
        Some(Shown { hide_goals: true })
    );
    settings
        .set(&SHOWN, &Shown { hide_goals: false })
        .await
        .unwrap();
    assert_eq!(
        settings
            .get_or(&SHOWN, Shown { hide_goals: true })
            .await
            .unwrap(),
        Shown { hide_goals: false }
    );
    assert!(settings.remove(&SHOWN).await.unwrap());
    assert!(!settings.remove(&SHOWN).await.unwrap());
    assert_eq!(settings.get(&SHOWN).await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_update_reads_and_writes_at_once_so_a_taken_entry_goes_to_one_caller() {
    let (_dir, db) = database().await;
    let links: SettingKey<Vec<String>> = SettingKey::new("links");
    let settings = db.settings();
    let added = settings
        .update(&links, Vec::new(), |all| {
            all.push("one".to_owned());
            all.len()
        })
        .await
        .unwrap();
    assert_eq!(added, 1, "the fallback stands in for a value never set");
    let takers = (0..8).map(|_| {
        let settings = db.settings();
        let links = links.clone();
        tokio::spawn(async move { settings.update(&links, Vec::new(), Vec::pop).await.unwrap() })
    });
    let mut taken = Vec::new();
    for taker in takers {
        taken.extend(taker.await.unwrap());
    }
    assert_eq!(taken, ["one"]);
    assert_eq!(settings.get(&links).await.unwrap(), Some(Vec::new()));
}

#[tokio::test]
async fn null_is_no_value_and_a_value_of_another_type_is_an_error() {
    let (_dir, db) = database().await;
    let settings = db.settings();
    let raw: SettingKey<Option<u32>> = SettingKey::new("shown");
    settings.set(&raw, &None).await.unwrap();
    assert_eq!(settings.get(&SHOWN).await.unwrap(), None);
    settings.set(&raw, &Some(3)).await.unwrap();
    assert!(matches!(
        settings.get(&SHOWN).await,
        Err(DbError::Setting { key, .. }) if key == "shown"
    ));
    let per_person: SettingKey<u32> = SettingKey::named(format!("count:{}", "guest"));
    settings.set(&per_person, &7).await.unwrap();
    assert_eq!(settings.get(&per_person).await.unwrap(), Some(7));
}

#[tokio::test]
async fn the_log_lists_the_newest_first() {
    let (_dir, db) = database().await;
    let log = db.log();
    let first = log.add("Paused the assistant", at(1)).await.unwrap();
    log.add("Resumed it", at(2)).await.unwrap();
    let lines = log.recent(40).await.unwrap();
    let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(texts, ["Resumed it", "Paused the assistant"]);
    assert_eq!(lines[1], first);
    assert_eq!(lines[1].at, at(1));
    assert_eq!(log.recent(1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_event_with_a_key_happens_once() {
    let (_dir, db) = database().await;
    let events = db.events();
    let event = NewEvent::new(RoomId::MAIN, "plan", "The build is ready").once("build:7");
    let stored = events.add(event.clone(), at(1)).await.unwrap().unwrap();
    assert_eq!(stored.key.as_deref(), Some("build:7"));
    assert_eq!(stored.at, at(1));
    assert!(!stored.handled);
    assert_eq!(events.add(event, at(2)).await.unwrap(), None);
    assert!(events.exists("build:7").await.unwrap());
    assert!(!events.exists("build:8").await.unwrap());
    // Without a key, every event is stored.
    let plain = NewEvent::new(RoomId::MAIN, "plan", "Again");
    assert!(events.add(plain.clone(), at(3)).await.unwrap().is_some());
    assert!(events.add(plain, at(4)).await.unwrap().is_some());
    assert_eq!(events.count_waiting().await.unwrap(), 3);
}

#[tokio::test]
async fn a_room_reads_its_waiting_events_once() {
    let (_dir, db) = database().await;
    let events = db.events();
    let first = events
        .add(NewEvent::new(RoomId::MAIN, "fact", "One"), at(1))
        .await
        .unwrap()
        .unwrap();
    let mut quiet = NewEvent::new(RoomId::MAIN, "fact", "Kept quiet");
    quiet.handled = true;
    events.add(quiet, at(2)).await.unwrap();
    let second = events
        .add(NewEvent::new(RoomId::MAIN, "whatsapp", "Two"), at(3))
        .await
        .unwrap()
        .unwrap();
    let waiting = events.waiting(RoomId::MAIN, 100).await.unwrap();
    assert_eq!(waiting, [first.clone(), second.clone()]);
    assert_eq!(
        events.handle_through(RoomId::MAIN, first.id).await.unwrap(),
        1
    );
    assert_eq!(events.waiting(RoomId::MAIN, 100).await.unwrap(), [second]);
    assert_eq!(events.handle_kind("whatsapp").await.unwrap(), 1);
    assert_eq!(events.count_waiting().await.unwrap(), 0);
    // An event for a room that does not exist is refused.
    assert!(
        events
            .add(NewEvent::new(RoomId::new(99), "fact", "Nowhere"), at(4))
            .await
            .is_err()
    );
}
