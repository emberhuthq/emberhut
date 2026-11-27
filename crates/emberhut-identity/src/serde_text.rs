//! Serde support (the `serde` feature): every value is its text, as `Display` writes it and
//! `FromStr` reads it, so deserializing applies the same checks as parsing.

use std::fmt::Display;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    AccountId, Host, LocalAddr, MachineId, MachineName, Nonce, ProbeKind, PublicKey, RelayDomain,
    Signature, Via,
};

macro_rules! as_text {
    ($($name:ty),+ $(,)?) => {$(
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                parse(deserializer)
            }
        }
    )+};
}

as_text!(
    AccountId,
    Host,
    LocalAddr,
    MachineId,
    MachineName,
    Nonce,
    ProbeKind,
    PublicKey,
    RelayDomain,
    Signature,
    Via,
);

fn parse<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
    text.parse().map_err(D::Error::custom)
}

#[cfg(test)]
mod tests {
    use crate::{MachineId, PublicKey, SecretKey, UnixMillis};

    #[test]
    fn values_are_their_text_and_are_checked() {
        let key = SecretKey::from_bytes(&[1; 32]).public_key();
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, format!("\"{key}\""));
        assert_eq!(serde_json::from_str::<PublicKey>(&json).unwrap(), key);
        let id = key.machine_id();
        assert_eq!(
            serde_json::from_str::<MachineId>(&format!("\"{id}\"")).unwrap(),
            id
        );
        let error =
            serde_json::from_str::<MachineId>(&format!("\"{}\"", id.to_string().to_uppercase()))
                .unwrap_err();
        assert!(error.to_string().contains("not in the alphabet"), "{error}");
        assert_eq!(
            serde_json::to_string(&UnixMillis::new(5).unwrap()).unwrap(),
            "5"
        );
        assert!(serde_json::from_str::<UnixMillis>("9007199254740992").is_err());
    }
}
