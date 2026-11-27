//! The guard rails around what the assistant and its workers do: which shell steps reach
//! outside the job ([`outward`], over [`shell`]), the owner's OK in one model ([`grants`]), their
//! go in the chat ([`chat_ok`], [`go`]), a card's Send it ([`send_tap`]), the final steps' Go
//! ahead cards ([`go_ahead`]), the ask guard ([`ask_guard`]), the words that make a step the
//! owner's every time ([`hard_words`]), and keys masked wherever they would show
//! ([`key_mask`]). Rules only: where OKs are kept and who is asked is the server's.

pub mod ask_guard;
pub mod chat_ok;
pub mod go;
pub mod go_ahead;
pub mod grants;
pub mod hard_words;
pub mod key_mask;
pub mod outward;
pub mod send_tap;
pub mod shell;
