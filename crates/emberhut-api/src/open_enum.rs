//! Server vocabularies that may grow.
//!
//! Some fields hold one of a few known words (a task's status, a theme's appearance). A newer
//! server can add a word, and an older client must still read the rest of the answer, so each
//! such type is an enum of the known words plus `Other(String)` for the rest, kept as sent and
//! written back unchanged.
//!
//! A word is a string literal, or a constant (the wire words live in [`crate::wire`]).

/// Defines an enum of known wire words with an `Other(String)` fallback, serialized as the word.
/// The generated type documents how to build it.
macro_rules! open_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $wire:expr, )+
        }
    ) => {
        $(#[$meta])*
        ///
        /// A word this contract doesn't know reads as `Other`, kept as sent. Build values from text
        /// with `From<&str>` or `From<String>`: `Other` holding a known word compares unequal to
        /// that word's variant. New words may become variants, hence `#[non_exhaustive]`.
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        $vis enum $name {
            $( $(#[$vmeta])* $variant, )+
            /// A word this version of the contract doesn't know, as the server sent it.
            Other(String),
        }

        impl $name {
            /// Every known word, in the order this contract lists them.
            $vis const KNOWN: &'static [&'static str] = &[$($wire),+];

            /// The word as it is on the wire.
            #[must_use]
            $vis fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $wire,)+
                    Self::Other(word) => word,
                }
            }

            /// Whether this is one of the known words (not [`Self::Other`]).
            #[must_use]
            $vis fn is_known(&self) -> bool {
                !matches!(self, Self::Other(_))
            }
        }

        impl $name {
            /// The known word `word` is, if it is one.
            fn known(word: &str) -> Option<Self> {
                $(
                    if word == $wire {
                        return Some(Self::$variant);
                    }
                )+
                None
            }
        }

        impl From<&str> for $name {
            fn from(word: &str) -> Self {
                Self::known(word).unwrap_or_else(|| Self::Other(word.to_owned()))
            }
        }

        impl From<String> for $name {
            fn from(word: String) -> Self {
                Self::known(&word).unwrap_or(Self::Other(word))
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer).map(Self::from)
            }
        }

        #[cfg(feature = "openapi")]
        impl ::utoipa::PartialSchema for $name {
            fn schema() -> ::utoipa::openapi::RefOr<::utoipa::openapi::schema::Schema> {
                ::utoipa::openapi::ObjectBuilder::new()
                    .schema_type(::utoipa::openapi::schema::Type::String)
                    .description(Some(format!(
                        "One of: {}. A newer server may send another word; clients keep it as it is.",
                        Self::KNOWN.join(", ")
                    )))
                    .examples([$($wire),+])
                    .into()
            }
        }

        #[cfg(feature = "openapi")]
        impl ::utoipa::ToSchema for $name {}
    };
}

pub(crate) use open_enum;

#[cfg(test)]
mod tests {
    open_enum! {
        /// A test vocabulary.
        pub(crate) enum Colour {
            /// Red.
            Red = "red",
            /// Blue.
            Blue = "blue",
            /// A word kept in a constant.
            Teal = TEAL,
        }
    }

    const TEAL: &str = "teal";

    #[test]
    fn known_words_read_as_their_variant() {
        assert_eq!(Colour::from("red"), Colour::Red);
        assert_eq!(Colour::from("blue".to_owned()), Colour::Blue);
        assert_eq!(
            serde_json::from_str::<Colour>(r#""blue""#).unwrap(),
            Colour::Blue
        );
        assert!(Colour::Red.is_known());
        assert_eq!(Colour::KNOWN, ["red", "blue", "teal"]);
        assert_eq!(Colour::from(TEAL), Colour::Teal);
        assert_eq!(Colour::Teal.as_str(), "teal");
    }

    #[test]
    fn unknown_words_are_kept_and_written_back() {
        let c: Colour = serde_json::from_str(r#""ultraviolet""#).unwrap();
        assert_eq!(c, Colour::Other("ultraviolet".to_owned()));
        assert!(!c.is_known());
        assert_eq!(serde_json::to_string(&c).unwrap(), r#""ultraviolet""#);
        assert_eq!(Colour::from("green").to_string(), "green");
        assert_eq!(Colour::from("green".to_owned()).as_str(), "green");
    }
}
