//! Between the domain's types (`emberhut-core`) and the contract's (`emberhut-api`).
//!
//! Both crates have their own id newtypes, and neither depends on the other, so the orphan
//! rule leaves no `From` between them: the conversions go through `i64` here. Instants become
//! the contract's text, JavaScript's `toISOString()`: UTC with milliseconds.

use emberhut_api as api;
use jiff::Timestamp;

/// The contract's room id for a domain one.
pub(crate) fn room_to_api(room: emberhut_core::RoomId) -> api::ids::RoomId {
    api::ids::RoomId::from(i64::from(room))
}

/// The domain's room id for a contract one.
pub(crate) fn room_from_api(room: api::ids::RoomId) -> emberhut_core::RoomId {
    emberhut_core::RoomId::from(i64::from(room))
}

/// The contract's message id for a domain one.
pub(crate) fn message_to_api(message: emberhut_core::MessageId) -> api::ids::MessageId {
    api::ids::MessageId::from(i64::from(message))
}

/// The domain's message id for a contract one.
pub(crate) fn message_from_api(message: api::ids::MessageId) -> emberhut_core::MessageId {
    emberhut_core::MessageId::from(i64::from(message))
}

/// The contract's device id for a domain one.
pub(crate) fn device_to_api(device: emberhut_core::DeviceId) -> api::ids::DeviceId {
    api::ids::DeviceId::from(i64::from(device))
}

/// An instant as the contract writes it: `2026-11-24T21:57:53.410Z`.
pub(crate) fn timestamp_to_api(instant: Timestamp) -> api::time::Timestamp {
    api::time::Timestamp::new(instant.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;

    #[test]
    fn instants_are_written_like_to_iso_string() {
        let instant =
            Timestamp::from_second(1_795_643_873).unwrap() + SignedDuration::from_millis(41);
        assert_eq!(
            timestamp_to_api(instant).as_str(),
            "2026-11-25T21:57:53.041Z"
        );
        let whole = Timestamp::from_second(1_795_643_873).unwrap();
        assert_eq!(timestamp_to_api(whole).as_str(), "2026-11-25T21:57:53.000Z");
    }

    #[test]
    fn ids_keep_their_number() {
        let room = emberhut_core::RoomId::new(4);
        assert_eq!(room_from_api(room_to_api(room)), room);
        let message = emberhut_core::MessageId::new(9);
        assert_eq!(message_from_api(message_to_api(message)), message);
        assert_eq!(device_to_api(emberhut_core::DeviceId::new(3)).get(), 3);
    }
}
