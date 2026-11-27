//! The directory's tables, as `SeaORM` entities: one module per table.
//!
//! The schema itself is made by [`super::migration`]; these mirror it for typed queries.

pub(super) mod accepted_signature;
pub(super) mod account;
pub(super) mod latest_statement;
pub(super) mod machine;
