//! How a message's text reads in the chat, with no GPUI in it: cut into blocks (quotes, tables,
//! pictures, the markdown around them), its task keys and bare addresses linked, a table laid
//! out as a grid or as cards, a run of the server's event lines summed up, the text the
//! message menu copies or sends, and all of it read once per message ([`parsed`]).

pub(crate) mod blocks;
pub(crate) mod copy;
pub(crate) mod direction;
pub(crate) mod events;
pub(crate) mod links;
pub(crate) mod literal;
pub(crate) mod parsed;
mod scan;
pub(crate) mod tables;
