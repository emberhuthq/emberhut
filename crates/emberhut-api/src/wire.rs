//! The wire words: values the contract spells in one place.
//!
//! The modules that use one name it under its own name and point here
//! ([`crate::headers::LOOKING`], [`crate::mcp::SERVER_NAME`], [`crate::paths::MCP_ROOM`],
//! [`crate::push::ANDROID_CHANNEL`], [`crate::threads::ACTIVITY`]), or, in the server, name
//! them from here. The MCP endpoint's values are in [`mcp`], a runner session's meta keys in
//! [`session_meta`].
//!
//! Wire enum words name their constant here too (the assistant's and the owner's words). A
//! serde field name has to be written out where the field is: the state's running-threads key
//! ([`crate::state::State::t3_running`]), and the assistant's account
//! ([`crate::state::State::assistant_account`]), which a test here checks against
//! [`ASSISTANT_ACCOUNT_FIELD`].

pub mod mcp;
pub mod session_meta;

/// The assistant: a session message's sender, a note's author, where a task's message went and
/// the owner of the assistant's own tasks.
pub const ASSISTANT: &str = "assistant";

/// The person the assistant works for: a session message's sender, a note's author, who drives
/// the browser or a browser profile.
pub const PERSON: &str = "owner";

/// The state's field for the account the assistant runs on
/// ([`crate::state::State::assistant_account`]).
pub const ASSISTANT_ACCOUNT_FIELD: &str = "assistantAccount";

/// The header that says a room is on screen.
pub const LOOKING_HEADER: &str = "x-emberhut-looking";

/// The Android notification channel push alerts name, which the Android app creates.
pub const ANDROID_CHANNEL: &str = "emberhut";

/// The path of a coding-thread app thread's latest steps.
pub const THREAD_ACTIVITY_PATH: &str = "/api/t3/activity";

/// Who a turn sent to a runner's session is from, when the assistant sent it: the word the
/// runners read.
pub const RUNNER_BY_ASSISTANT: &str = "assistant";

/// A runner the server's configuration names (`--runner NAME=URL`), as `GET /api/runners`
/// says where it knows the runner from.
pub const RUNNER_FROM_FLAG: &str = "flag";

/// A runner paired at run time (`POST /api/runner/pair`), as `GET /api/runners` says where it
/// knows the runner from.
pub const RUNNER_PAIRED: &str = "paired";

/// The name of a voice session's one tool, which the phone answers.
pub const VOICE_ASK_TOOL: &str = "ask_assistant";

/// The label a client gives the person's lines in a voice call's transcript
/// (`POST /api/voice/done`'s `transcript`).
pub const VOICE_PERSON_LABEL: &str = "Owner:";

/// The server's own browser, as the browser tools' `browser` argument names it (agents' prompts
/// and tasks carry it).
pub const OWN_BROWSER: &str = "assistant";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_names_the_assistants_account_by_its_wire_field() {
        let body = r#"{
            "room": 1, "rooms": [], "on": true, "busy": false, "live": null, "unread": false,
            "pending": 0, "chat": [], "earlier": false
        }"#;
        let mut state: crate::state::State = serde_json::from_str(body).unwrap();
        state.assistant_account =
            Some(serde_json::from_value(serde_json::json!({"on": "a-1", "mode": "auto"})).unwrap());
        let written = serde_json::to_value(&state).unwrap();
        assert!(written.get(ASSISTANT_ACCOUNT_FIELD).is_some(), "{written}");
    }
}
