//! The setup checklist: read with the session, each item as the server words it.

use emberhut_api::setup::{SetupItemId, SetupState};
use serde_json::json;

use crate::support::{Reply, TestServer, fresh};

#[test]
fn the_checklist_is_read_item_by_item() {
    let server = TestServer::start(|_| {
        Reply::Json(
            200,
            json!({ "items": [
                { "id": "provider", "state": "missing" },
                { "id": "chrome", "state": "ready", "detail": "1 profile paired, 1 online." },
                { "id": "a-later-item", "state": "a-later-state" }
            ] }),
        )
    });
    let page = server.client(fresh("r1")).setup().unwrap();
    assert_eq!(
        page.item(&SetupItemId::Provider).map(|item| &item.state),
        Some(&SetupState::Missing)
    );
    assert_eq!(
        page.item(&SetupItemId::Chrome).map(|item| &item.state),
        Some(&SetupState::Ready)
    );
    assert_eq!(
        page.items.len(),
        3,
        "an item this client doesn't know is kept"
    );
    let asked = server.requests_to("/api/setup");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].method, "GET");
}
