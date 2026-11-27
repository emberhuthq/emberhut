//! The owner's Gmail, read only: `gmail_connect`'s one-time link, `GET /gmail/connect/{token}`
//! on to Google's consent screen, `GET /gmail/callback` saving the account's sign-in, the
//! first look at its mail, and the messages tools and `replied` reading it.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure fails the test that called them"
)]

use serde_json::{Value, json};

use crate::common::{Call, text};
use crate::messaging_fakes::Kit;

/// A Gmail message from `from` to `to` with `labels`, `minutes` before now.
fn message(
    kit: &Kit,
    id: &str,
    from: &str,
    to: &str,
    labels: &[&str],
    minutes: i64,
    body: &str,
) -> Value {
    use base64::Engine as _;
    let at = (kit.now() - jiff::SignedDuration::from_mins(minutes)).as_millisecond();
    json!({
        "id": id,
        "threadId": "t1",
        "labelIds": labels,
        "internalDate": at.to_string(),
        "snippet": "a snippet",
        "payload": {
            "mimeType": "text/plain",
            "headers": [
                { "name": "From", "value": from },
                { "name": "To", "value": to },
                { "name": "Subject", "value": format!("About {id}") }
            ],
            "body": { "data": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body) }
        }
    })
}

/// The link's token in a `gmail_connect` answer.
fn token_of(answer: &str) -> String {
    let start = answer.find("/gmail/connect/").unwrap() + "/gmail/connect/".len();
    answer[start..]
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one account's whole way: the link, Google's screen, the callback, the first look, the reads"
)]
pub(crate) async fn an_account_is_signed_in_through_google_and_its_mail_is_read() {
    let kit = Kit::new().await;
    let early = kit
        .app
        .call_tool("gmail_connect", json!({ "account": "home" }))
        .await;
    assert!(
        text(&early).starts_with("Can't yet: the server's Google OAuth client isn't saved yet")
    );

    kit.secret("gmail-client-id", "client-1");
    kit.secret("gmail-client-secret", "shh");
    let answer = kit
        .app
        .call_tool(
            "gmail_connect",
            json!({ "account": "home", "email": "me@example.com" }),
        )
        .await;
    let answer = text(&answer);
    assert!(
        answer.starts_with("https://box.example/gmail/connect/"),
        "{answer}"
    );
    assert!(answer.ends_with("(works once, for a day)"));
    let token = token_of(answer);

    let consent = kit
        .app
        .call(Call::get(&format!("/gmail/connect/{token}")))
        .await;
    assert_eq!(consent.status, 302);
    let to = consent.header("location");
    assert!(
        to.starts_with("https://consent.example/auth?client_id=client-1"),
        "{to}"
    );
    assert!(
        to.contains("redirect_uri=https%3A%2F%2Fbox.example%2Fgmail%2Fcallback"),
        "{to}"
    );
    assert!(to.contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.readonly"));
    assert!(to.contains("login_hint=me%40example.com"));
    assert!(to.contains(&format!("state={token}")));

    kit.google.token(
        200,
        json!({
            "refresh_token": "refresh-1", "access_token": "access-1", "expires_in": 3600,
            "scope": "https://www.googleapis.com/auth/gmail.readonly"
        }),
    );
    kit.google.get(
        "profile",
        200,
        json!({ "emailAddress": "me@example.com", "historyId": "500" }),
    );
    let ann = message(
        &kit,
        "m1",
        "Ann Lee <ann@example.com>",
        "me@example.com",
        &["INBOX"],
        30,
        "Lunch Friday?\n\nOn Mon, 3 Nov 2026 at 09:12, Me <me@example.com> wrote:\n> earlier",
    );
    let mine = message(
        &kit,
        "m2",
        "me@example.com",
        "Ann Lee <ann@example.com>",
        &["SENT"],
        10,
        "Friday works.",
    );
    let promo = message(
        &kit,
        "m3",
        "deals@example.net",
        "me@example.com",
        &["INBOX", "CATEGORY_PROMOTIONS"],
        20,
        "50% off",
    );
    kit.google.get(
        "messages",
        200,
        json!({ "messages": [{ "id": "m1" }, { "id": "m2" }, { "id": "m3" }] }),
    );
    for m in [&ann, &mine, &promo] {
        let id = m["id"].as_str().unwrap();
        kit.google.get(&format!("messages/{id}"), 200, m.clone());
    }
    let back = kit
        .app
        .call(Call::get("/gmail/callback").query(&format!("state={token}&code=code-1")))
        .await;
    assert_eq!(back.status, 200, "{}", back.text);
    assert!(back.text.contains("me@example.com"));
    assert_eq!(
        kit.read_secret("gmail-home-refresh").as_deref(),
        Some("refresh-1")
    );
    let exchange = &kit.google.forms()[0];
    assert!(exchange.contains(&("code".to_owned(), "code-1".to_owned())));
    assert!(exchange.contains(&("grant_type".to_owned(), "authorization_code".to_owned())));
    let used = kit
        .app
        .call(Call::get("/gmail/callback").query(&format!("state={token}&code=code-1")))
        .await;
    assert_eq!(used.status, 404, "the link works once");

    kit.app.state.messaging().gmail().collect().await;
    assert!(
        kit.google
            .paths()
            .iter()
            .any(|p| p.starts_with("messages?maxResults=100&q=newer_than%3A7d")),
        "the first look reads the last week: {:?}",
        kit.google.paths()
    );
    let chats = kit.app.call_tool("messages_chats", json!({})).await;
    let chats = text(&chats);
    assert!(
        chats.contains("Email (Email Home: me@example.com)"),
        "{chats}"
    );
    assert!(
        chats.contains(r#""id":"email:home:ann@example.com""#),
        "{chats}"
    );
    assert!(
        !chats.contains("deals@example.net"),
        "quiet mail doesn't list: {chats}"
    );

    let read = kit
        .app
        .call_tool(
            "messages_read",
            json!({ "chat": "email:home:ann@example.com" }),
        )
        .await;
    let read: Value = serde_json::from_str(text(&read)).unwrap();
    assert_eq!(
        read["conversation"],
        "Ann Lee <ann@example.com> (Email Home, me@example.com)"
    );
    assert_eq!(read["messages"].as_array().unwrap().len(), 2);
    let first = read["messages"][0].as_str().unwrap();
    assert!(
        first.ends_with("Ann Lee <ann@example.com>: «About m1» Lunch Friday?"),
        "the quoted mail is cut: {first}"
    );
    assert!(
        read["messages"][1]
            .as_str()
            .unwrap()
            .contains("Owner: «About m2» Friday works.")
    );

    let since = (kit.now() - jiff::SignedDuration::from_hours(1)).to_string();
    let replied = kit
        .app
        .call_tool(
            "replied",
            json!({ "chat": "email:home:ann@example.com", "since": since }),
        )
        .await;
    let replied: Value = serde_json::from_str(text(&replied)).unwrap();
    assert_eq!(replied["answered"], true);
    assert!(
        replied["his"][0]
            .as_str()
            .unwrap()
            .contains("Friday works.")
    );

    kit.google
        .get("messages", 200, json!({ "messages": [{ "id": "m2" }] }));
    kit.mac.answers("/search", &json!([]));
    let found = kit
        .app
        .call_tool("messages_search", json!({ "q": "friday" }))
        .await;
    let found = text(&found);
    assert!(
        found.contains(r#""subject":"About m2""#),
        "Gmail's own search: {found}"
    );
    assert!(found.contains(r#""sender":"Owner""#), "{found}");
}

#[tokio::test]
pub(crate) async fn a_sign_in_with_another_address_or_without_the_scope_saves_nothing() {
    let kit = Kit::new().await;
    kit.secret("gmail-client-id", "client-1");
    kit.secret("gmail-client-secret", "shh");
    let link = kit
        .app
        .call_tool(
            "gmail_connect",
            json!({ "account": "work", "email": "me@example.net" }),
        )
        .await;
    let token = token_of(text(&link));
    kit.google.token(
        200,
        json!({ "refresh_token": "r", "access_token": "a", "scope": "https://www.googleapis.com/auth/gmail.readonly" }),
    );
    kit.google.get(
        "profile",
        200,
        json!({ "emailAddress": "other@example.com" }),
    );
    let wrong = kit
        .app
        .call(Call::get("/gmail/callback").query(&format!("state={token}&code=c")))
        .await;
    assert_eq!(wrong.status, 400);
    assert!(
        wrong
            .text
            .contains("That was other@example.com, not me@example.net")
    );
    assert_eq!(kit.read_secret("gmail-work-refresh"), None);

    let refused = kit
        .app
        .call(Call::get("/gmail/callback").query("state=unknown&error=access_denied"))
        .await;
    assert_eq!(refused.status, 404);
    let gone = kit
        .app
        .call(Call::get("/gmail/connect/not-a-link-at-all-123"))
        .await;
    assert_eq!(gone.status, 404);
    assert!(gone.text.contains("expired"));

    let link = kit
        .app
        .call_tool("gmail_connect", json!({ "account": "work" }))
        .await;
    let token = token_of(text(&link));
    let denied = kit
        .app
        .call(Call::get("/gmail/callback").query(&format!("state={token}&error=access_denied")))
        .await;
    assert_eq!(denied.status, 400);
    assert!(denied.text.contains("Google said access_denied"));

    let link = kit
        .app
        .call_tool("gmail_connect", json!({ "account": "work" }))
        .await;
    let token = token_of(text(&link));
    kit.google
        .get("profile", 200, json!({ "emailAddress": "me@example.net" }));
    std::fs::create_dir_all(kit.secrets.join("gmail-work-refresh")).unwrap();
    let unsaved = kit
        .app
        .call(Call::get("/gmail/callback").query(&format!("state={token}&code=c")))
        .await;
    assert_eq!(unsaved.status, 500, "{}", unsaved.text);
    assert!(unsaved.text.contains("couldn't be saved"));

    let bad = kit
        .app
        .call_tool("gmail_connect", json!({ "account": "Work!" }))
        .await;
    assert!(
        text(&bad).starts_with("Can't yet: account:"),
        "{}",
        text(&bad)
    );
}

#[tokio::test]
pub(crate) async fn mail_reads_fall_back_to_what_is_kept_when_google_refuses() {
    let kit = Kit::new().await;
    kit.secret("gmail-home-refresh", "refresh-1");
    kit.google.token(400, json!({ "error": "invalid_grant" }));
    kit.app.state.messaging().gmail().collect().await;
    let chats = kit.app.call_tool("messages_chats", json!({})).await;
    assert!(
        text(&chats)
            .contains("not read now: Google no longer takes home's sign-in: gmail_connect again"),
        "{}",
        text(&chats)
    );
    let read = kit
        .app
        .call_tool(
            "messages_read",
            json!({ "chat": "email:home:ann@example.com" }),
        )
        .await;
    let read: Value = serde_json::from_str(text(&read)).unwrap();
    assert!(
        read["note"]
            .as_str()
            .unwrap()
            .starts_with("Gmail didn't answer"),
        "{read}"
    );
    let bad = kit
        .app
        .call_tool("messages_read", json!({ "chat": "email:nope" }))
        .await;
    assert!(text(&bad).contains("isn't an email chat id"));
}

#[tokio::test]
async fn a_sign_in_link_works_once_even_for_two_callbacks_at_once_and_not_after_a_day() {
    let exchanges = |kit: &Kit| {
        kit.google
            .forms()
            .iter()
            .filter(|form| {
                form.contains(&("grant_type".to_owned(), "authorization_code".to_owned()))
            })
            .count()
    };
    let kit = Kit::new().await;
    kit.secret("gmail-client-id", "client-1");
    kit.secret("gmail-client-secret", "shh");
    kit.google.token(
        200,
        json!({ "refresh_token": "r", "access_token": "a", "scope": "https://www.googleapis.com/auth/gmail.readonly" }),
    );
    kit.google
        .get("profile", 200, json!({ "emailAddress": "me@example.net" }));
    kit.google.get("messages", 200, json!({ "messages": [] }));

    let link = kit
        .app
        .call_tool("gmail_connect", json!({ "account": "work" }))
        .await;
    let token = token_of(text(&link));
    let back = Call::get("/gmail/callback").query(&format!("state={token}&code=c"));
    let again = Call::get("/gmail/callback").query(&format!("state={token}&code=c"));
    let (one, two) = tokio::join!(kit.app.call(back), kit.app.call(again));
    let mut statuses = [one.status, two.status];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 404], "{} / {}", one.text, two.text);
    assert_eq!(exchanges(&kit), 1, "one code exchange");

    let link = kit
        .app
        .call_tool("gmail_connect", json!({ "account": "home" }))
        .await;
    let token = token_of(text(&link));
    kit.later(24 * 60 * 60 + 1);
    let page = kit
        .app
        .call(Call::get(&format!("/gmail/connect/{token}")))
        .await;
    assert_eq!(page.status, 404);
    let late = kit
        .app
        .call(Call::get("/gmail/callback").query(&format!("state={token}&code=c")))
        .await;
    assert_eq!(late.status, 404);
    assert!(late.text.contains("expired"), "{}", late.text);
    assert_eq!(exchanges(&kit), 1, "no exchange for an expired link");
    assert_eq!(kit.read_secret("gmail-home-refresh"), None);
}
