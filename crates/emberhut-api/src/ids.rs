//! Identifier newtypes.
//!
//! Every id on the wire is a bare JSON number or string; here each kind has its own type, so a
//! room id can't be passed where a task id is expected. All of them are `#[serde(transparent)]`:
//! the JSON is unchanged.
//!
//! Numeric ids are database row ids, so a valid one is positive; with the `validate` feature the
//! request types check that.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Defines a numeric id newtype over `i64`.
macro_rules! numeric_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
        #[cfg_attr(feature = "validate", derive(garde::Validate), garde(transparent))]
        pub struct $name(
            /// Row ids start at 1.
            #[cfg_attr(feature = "validate", garde(range(min = 1)))]
            i64,
        );

        impl $name {
            /// Wraps a raw id.
            #[must_use]
            pub const fn new(id: i64) -> Self {
                Self(id)
            }

            /// The raw id.
            #[must_use]
            pub const fn get(self) -> i64 {
                self.0
            }
        }

        impl From<i64> for $name {
            fn from(id: i64) -> Self {
                Self(id)
            }
        }

        impl From<$name> for i64 {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

/// Defines a textual id newtype over `String`.
macro_rules! text_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
        pub struct $name(String);

        impl $name {
            /// Wraps a raw id.
            #[must_use]
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// The raw id.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// The raw id, owned.
            #[must_use]
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self(id)
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

numeric_id! {
    /// A room: Main, or one conversation per project.
    RoomId
}

impl RoomId {
    /// Main, the room that exists before any project has one. A request without a room, or with
    /// one the server doesn't know, is about Main.
    pub const MAIN: Self = Self(1);

    /// Whether this is [`RoomId::MAIN`].
    #[must_use]
    pub const fn is_main(self) -> bool {
        self.0 == Self::MAIN.0
    }
}

impl Default for RoomId {
    /// [`RoomId::MAIN`], as the server reads a request that names no room.
    fn default() -> Self {
        Self::MAIN
    }
}

numeric_id! {
    /// A chat message. Ids grow with time within the whole chat, so they order a room's messages.
    MessageId
}

numeric_id! {
    /// A task (shown by its key: the server's task-key prefix followed by the task's number).
    TaskId
}

numeric_id! {
    /// A signed-in device's session row.
    DeviceId
}

numeric_id! {
    /// A project.
    ProjectId
}

numeric_id! {
    /// A goal.
    GoalId
}

numeric_id! {
    /// Something a task produced (a picture, a report, a link).
    ArtifactId
}

text_id! {
    /// A machine that runs sessions, as its runner names itself (for example `runner`).
    MachineId
}

text_id! {
    /// A session on a runner.
    SessionId
}

text_id! {
    /// A row of work in flight (`working` in the state): a task's or a thread's.
    WorkId
}

text_id! {
    /// A call recorded on the Mac the server works with.
    CallId
}

numeric_id! {
    /// One run of work on a task: a turn of a thread, or a worker's session.
    RunId
}

numeric_id! {
    /// A sub-agent of a task's run.
    AgentId
}

/// This installation of a client: made once and kept with its session, so signing in again on
/// the same device replaces that device's row on the server instead of adding one.
///
/// Written out rather than by the text-id macro only for its validation rule; its API is the
/// other text ids'.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate), garde(transparent))]
pub struct InstallId(
    /// 1 to 64 UTF-16 units.
    #[cfg_attr(feature = "validate", garde(length(utf16, min = 1, max = 64)))]
    String,
);

impl InstallId {
    /// The longest install id the server keeps, in UTF-16 units.
    pub const MAX_LEN: usize = 64;

    /// Wraps a raw id.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The raw id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The raw id, owned.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl From<String> for InstallId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for InstallId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl AsRef<str> for InstallId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InstallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

numeric_id! {
    /// A rule on trial: the lesson of one of the person's corrections, followed for a while
    /// before a review keeps or drops it.
    TrialId
}

numeric_id! {
    /// A standing rule the person agreed to.
    LessonId
}

numeric_id! {
    /// Something the assistant watches for the person (a page, a reply), until it is stopped.
    WatchId
}

numeric_id! {
    /// A paired Chrome profile.
    ChromeProfileId
}

text_id! {
    /// A fact the assistant keeps about the person: its category and its name, such as
    /// `people/example-person`.
    FactId
}

text_id! {
    /// A background job the assistant started (its own id, such as `b1a2`).
    JobId
}

text_id! {
    /// A Chrome pairing link's code, as the link shows it.
    ChromeCode
}

text_id! {
    /// A configured agent: a slug such as `client-a`; `main` is the default agent. Not a
    /// task's sub-agent, which is an [`AgentId`].
    ConfiguredAgentId
}

impl ConfiguredAgentId {
    /// The default agent's id.
    pub const MAIN: &'static str = "main";

    /// Whether this is the default agent.
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.0 == Self::MAIN
    }
}

text_id! {
    /// An external approver: a slug such as `example-lead`.
    ApproverId
}

numeric_id! {
    /// An approval request.
    ApprovalId
}

numeric_id! {
    /// An area of a project: a part of it, with its own tasks.
    AreaId
}

text_id! {
    /// One turn of a runner session: the session's id, a colon and the turn's number
    /// (`r-1a2b:3`).
    TurnId
}

impl TurnId {
    /// The turn's number, after the last colon; none when it has none.
    #[must_use]
    pub fn number(&self) -> Option<u32> {
        self.0.rsplit_once(':')?.1.parse().ok()
    }
}

text_id! {
    /// A tool call in a runner session's transcript: its result and a helper agent's own
    /// steps point back to it.
    ToolCallId
}

text_id! {
    /// A place the assistant works or a connection it reads through, on the environments page
    /// (`runner`, `browser`, `gmail:work`).
    EnvironmentId
}

text_id! {
    /// A job running on one of the machines, as the jobs list names it (`unit:<name>`,
    /// `pid:<n>`, `session:<id>`).
    MachineJobId
}

numeric_id! {
    /// A party: personas talking about the person's topic.
    PartyId
}

numeric_id! {
    /// A line said in a party.
    PartyLineId
}

text_id! {
    /// A persona: `p1` to `p5` in a party, a ready-made one's own id (`skeptic`) among the
    /// presets, `recent1`… among the person's own from earlier parties.
    PersonaId
}

numeric_id! {
    /// A message in the Dark room.
    DarkMessageId
}

numeric_id! {
    /// One of the tabs of the assistant's own browser, as the server numbers them.
    BrowserTabId
}

text_id! {
    /// One of the person's devices, as it names itself to the assistant's browser: one device
    /// drives while the others watch. An empty one, in the browser's state, means the
    /// assistant handed it over and no device has taken it yet.
    BrowserDeviceId
}

text_id! {
    /// A quick note, made by the device that wrote it (8 to 64 letters, digits, `_` or `-`),
    /// so sending it again changes nothing.
    NoteUid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_ids_are_bare_numbers_on_the_wire() {
        let room: RoomId = serde_json::from_str("7").unwrap();
        assert_eq!(room, RoomId::new(7));
        assert_eq!(serde_json::to_string(&TaskId::new(42)).unwrap(), "42");
        assert_eq!(i64::from(MessageId::from(5)), 5);
        assert_eq!(DeviceId::new(3).to_string(), "3");
    }

    #[test]
    fn main_is_room_one_and_the_default() {
        assert_eq!(RoomId::MAIN.get(), 1);
        assert_eq!(RoomId::default(), RoomId::MAIN);
        assert!(RoomId::MAIN.is_main());
        assert!(!RoomId::new(2).is_main());
    }

    #[test]
    fn text_ids_are_bare_strings_on_the_wire() {
        let machine: MachineId = serde_json::from_str(r#""runner""#).unwrap();
        assert_eq!(machine.as_str(), "runner");
        assert_eq!(machine.to_string(), "runner");
        assert_eq!(SessionId::from("s-1").as_ref(), "s-1");
        assert_eq!(WorkId::from("task:1".to_owned()).into_inner(), "task:1");
        let install = InstallId::from("zr-1".to_owned());
        assert_eq!(serde_json::to_string(&install).unwrap(), r#""zr-1""#);
        assert_eq!(install.to_string(), install.as_str());
        assert_eq!(InstallId::from("zr-1").as_ref(), "zr-1");
        assert_eq!(install.into_inner(), "zr-1");
    }
}
