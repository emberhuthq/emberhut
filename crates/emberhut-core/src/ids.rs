//! Row ids, one newtype per table, so an id of one kind cannot be passed where another is meant.

use std::fmt;

macro_rules! row_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(i64);

        impl $name {
            /// The id with this number.
            #[must_use]
            pub const fn new(id: i64) -> Self {
                Self(id)
            }

            /// The id's number, as the database and the API write it.
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

row_id!(
    /// A signed-in device: one per app install.
    DeviceId
);
row_id!(
    /// A one-time pairing code.
    PairingCodeId
);
row_id!(
    /// A room: one conversation.
    RoomId
);
row_id!(
    /// One message in a room.
    MessageId
);
row_id!(
    /// One line of the activity log.
    LogLineId
);
row_id!(
    /// Something a program or the server told the assistant.
    EventId
);
row_id!(
    /// A file stored for the chat.
    FileId
);
row_id!(
    /// A one-time secret link.
    SecretLinkId
);
row_id!(
    /// A party: AI personas talking about a topic.
    PartyId
);
row_id!(
    /// One line said in a party.
    PartyMessageId
);

row_id!(
    /// An approval request.
    ApprovalId
);

impl RoomId {
    /// Main, the room every server starts with.
    pub const MAIN: Self = Self(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_show_their_number() {
        assert_eq!(DeviceId::new(7).to_string(), "7");
        assert_eq!(RoomId::MAIN.get(), 1);
    }

    #[test]
    fn ids_convert_to_and_from_i64() {
        assert_eq!(RoomId::from(3), RoomId::new(3));
        assert_eq!(i64::from(MessageId::new(9)), 9);
    }
}
