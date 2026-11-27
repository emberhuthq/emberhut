//! Threads of the coding-thread app still running there, read from a client: listed read
//! only beside the runner sessions (the state's `t3Running`, [`ExternalThread`]) until they
//! finish, and one thread's latest steps ([`ThreadActivity`] at [`crate::threads::ACTIVITY`]),
//! which a client asks for again every few seconds while it shows them: the app can't stream them.

use crate::query::push_encoded;
#[cfg(doc)]
use crate::threads::ThreadActivity;
use crate::threads::{ActivityQuery, ActivityStep, ExternalThread};

/// The machine word of the Mac's coding-thread app, the only one not on the Linux PC (a wire value).
pub const T3_MAC: &str = "t3-mac";

impl ExternalThread {
    /// Whether it runs in the Mac's coding-thread app.
    #[must_use]
    pub fn on_mac(&self) -> bool {
        self.machine.as_str() == T3_MAC
    }

    /// Where it runs, in words.
    #[must_use]
    pub fn place(&self) -> &'static str {
        if self.on_mac() {
            "Coding app on the Mac"
        } else {
            "Coding app on the Linux PC"
        }
    }

    /// Whether it waits on the person.
    #[must_use]
    pub fn is_waiting(&self) -> bool {
        self.status == "waiting"
    }
}

impl ActivityQuery {
    /// The query string, without the `?`.
    ///
    /// ```
    /// use emberhut_api::threads::ActivityQuery;
    ///
    /// let query = ActivityQuery { machine: "t3-mac".into(), thread: "a b".into() };
    /// assert_eq!(query.to_query(), "machine=t3-mac&thread=a%20b");
    /// ```
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut query = String::from("machine=");
        push_encoded(&mut query, &self.machine);
        query.push_str("&thread=");
        push_encoded(&mut query, &self.thread);
        query
    }
}

impl ActivityStep {
    /// What it is, in a word for a person: Said, Asked, Ran, Thought, Tool, or the app's own word.
    #[must_use]
    pub fn kind_name(&self) -> String {
        match self.kind.as_str() {
            "assistant_message" => "Said".to_owned(),
            "user_message" => "Asked".to_owned(),
            "command_execution" => "Ran".to_owned(),
            "reasoning" => "Thought".to_owned(),
            "dynamic_tool" => "Tool".to_owned(),
            other => {
                let words = other.replace('_', " ");
                let mut chars = words.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                })
            }
        }
    }

    /// Whether it is a command (shown monospaced).
    #[must_use]
    pub fn is_command(&self) -> bool {
        self.kind == "command_execution"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: &str) -> ActivityStep {
        ActivityStep {
            kind: kind.into(),
            title: None,
            text: String::new(),
            at: None,
        }
    }

    #[test]
    fn a_steps_kind_reads_as_a_word() {
        assert_eq!(step("assistant_message").kind_name(), "Said");
        assert_eq!(step("file_change").kind_name(), "File change");
        assert!(step("command_execution").is_command());
    }
}
