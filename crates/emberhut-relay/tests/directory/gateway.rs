//! The gateways' interface, in-process: lookups, presence, and the unlinks pushed to them.

use emberhut_identity::{AccountId, MachineId};
use emberhut_relay::directory::{GatewayDirectory as _, MachineStatus, Presence};
use serde_json::json;

use crate::support::{ACCOUNT, Harness, VECTOR_MACHINE, VECTOR_TIME, machine_id, vector_link};

#[tokio::test]
async fn a_gateway_looks_machines_up_in_one_batch() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(harness.link(&token, &vector_link()).await.status, 200);
    let linked: MachineId = VECTOR_MACHINE.parse().unwrap();
    let stranger: MachineId = machine_id(9).parse().unwrap();
    let statuses = harness.gateway.lookup(&[stranger, linked]).await.unwrap();
    assert_eq!(
        statuses,
        [
            MachineStatus::NotLinked,
            MachineStatus::Linked {
                account: AccountId::new(ACCOUNT).unwrap()
            }
        ]
    );
    assert_eq!(harness.gateway.lookup(&[]).await.unwrap(), []);
}

#[tokio::test]
async fn presence_shows_in_the_accounts_list() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(harness.link(&token, &vector_link()).await.status, 200);
    let id: MachineId = VECTOR_MACHINE.parse().unwrap();
    harness
        .gateway
        .report(
            &id,
            Presence::Up {
                held_by: "box-1".to_owned(),
            },
        )
        .await
        .unwrap();
    harness.clock.set(VECTOR_TIME + 60_000);
    let token = harness.session(ACCOUNT, &json!({}));
    let up = harness.list(&token).await.body["machines"][0].clone();
    assert_eq!(
        (up["connected"].clone(), up["lastSeen"].clone()),
        (json!(true), json!(VECTOR_TIME + 60_000))
    );
    harness.gateway.report(&id, Presence::Down).await.unwrap();
    harness.clock.set(VECTOR_TIME + 120_000);
    let token = harness.session(ACCOUNT, &json!({}));
    let down = harness.list(&token).await.body["machines"][0].clone();
    assert_eq!(
        (down["connected"].clone(), down["lastSeen"].clone()),
        (json!(false), json!(VECTOR_TIME + 60_000))
    );
}

#[tokio::test]
async fn a_box_that_starts_again_drops_the_uplinks_it_held() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(harness.link(&token, &vector_link()).await.status, 200);
    let id: MachineId = VECTOR_MACHINE.parse().unwrap();
    harness
        .gateway
        .report(
            &id,
            Presence::Up {
                held_by: "box-1".to_owned(),
            },
        )
        .await
        .unwrap();
    // Another box starting changes nothing here.
    harness.gateway.started("box-2").await.unwrap();
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(
        harness.list(&token).await.body["machines"][0]["connected"],
        json!(true)
    );
    // The box that held it crashed and starts again.
    harness.clock.set(VECTOR_TIME + 30_000);
    harness.gateway.started("box-1").await.unwrap();
    let token = harness.session(ACCOUNT, &json!({}));
    let machine = harness.list(&token).await.body["machines"][0].clone();
    assert_eq!(
        (machine["connected"].clone(), machine["lastSeen"].clone()),
        (json!(false), json!(VECTOR_TIME + 30_000))
    );
}
