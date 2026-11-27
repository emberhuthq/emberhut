//! Closed vocabularies with fixed wire words: one enum per vocabulary, its words written once.

/// A word that is none of a vocabulary's.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{word:?} is not one of: {}", .known.join(", "))]
pub struct UnknownWord {
    /// The word as given.
    pub word: String,
    /// The vocabulary's words.
    pub known: &'static [&'static str],
}

/// Defines a fieldless enum whose variants each have one fixed wire word, with `ALL`,
/// `as_str`, `parse`, `Display` and `FromStr` (whose error is [`UnknownWord`]).
macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $wire:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant, )+
        }

        impl $name {
            /// Every value, in the order the vocabulary lists them.
            $vis const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Every wire word, in the same order.
            $vis const WORDS: &'static [&'static str] = &[$($wire),+];

            /// Its word, as the database and the API write it.
            #[must_use]
            $vis const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }

            /// The value whose word is `word`, if there is one.
            #[must_use]
            $vis fn parse(word: &str) -> Option<Self> {
                match word {
                    $($wire => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::wire::UnknownWord;

            fn from_str(word: &str) -> Result<Self, Self::Err> {
                Self::parse(word).ok_or_else(|| $crate::wire::UnknownWord {
                    word: word.to_owned(),
                    known: Self::WORDS,
                })
            }
        }
    };
}

pub(crate) use wire_enum;

#[cfg(test)]
mod tests {
    wire_enum! {
        /// A test vocabulary.
        pub(crate) enum Colour {
            /// Red.
            Red = "red",
            /// Dark blue.
            DarkBlue = "dark_blue",
        }
    }

    #[test]
    fn words_round_trip() {
        for colour in Colour::ALL {
            assert_eq!(Colour::parse(colour.as_str()), Some(*colour));
            assert_eq!(colour.to_string().parse::<Colour>(), Ok(*colour));
        }
        assert_eq!(Colour::WORDS, ["red", "dark_blue"]);
    }

    #[test]
    fn an_unknown_word_names_the_known_ones() {
        let error = "green".parse::<Colour>().unwrap_err();
        assert_eq!(error.to_string(), "\"green\" is not one of: red, dark_blue");
        assert_eq!(Colour::parse("Red"), None);
    }
}
