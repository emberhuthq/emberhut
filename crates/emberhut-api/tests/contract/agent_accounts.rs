//! The agent CLI accounts and the waitlists.

use emberhut_api::agent_accounts::AgentAccounts;
use emberhut_api::waitlist::{CLOUD_HOME, Waitlists};

use crate::support::{assert_round_trip, fixture, from_value, with_unknown_fields};

#[test]
fn reads_and_writes_the_accounts_and_tolerates_unknown_fields() {
    let page: AgentAccounts = assert_round_trip("agent_accounts.json");
    assert!(page.moves);
    let [first, second] = page.accounts.as_slice() else {
        panic!("two accounts");
    };
    assert!(!first.can_work);
    assert_eq!(
        first
            .resting_until
            .as_ref()
            .map(emberhut_api::time::Timestamp::as_str),
        Some("2030-03-17T17:46:40.000Z")
    );
    assert!(second.can_work && second.on);
    let mut body = fixture("agent_accounts.json");
    with_unknown_fields(&mut body);
    assert_eq!(from_value::<AgentAccounts>(body), page);
}

#[test]
fn reads_and_writes_the_waitlists() {
    let lists: Waitlists = assert_round_trip("waitlists.json");
    assert_eq!(lists.lists[0].id, CLOUD_HOME);
    assert!(lists.lists[0].joined);
}
