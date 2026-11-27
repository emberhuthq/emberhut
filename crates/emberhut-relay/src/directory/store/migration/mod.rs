//! The directory's schema history: every migration, in order. [`Store::open`](super::Store::open)
//! applies the ones a database lacks.
//!
//! A migration is never edited once it is on `main`; a change to the schema is a new one.
//! Migrations name tables and columns with their own identifiers rather than the entities',
//! so a later change to an entity cannot rewrite history.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::{MigrationTrait, MigratorTrait};

mod m20261126_000001_directory;

/// Every migration of the directory's database.
#[derive(Debug)]
pub(super) struct Migrator;

#[async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20261126_000001_directory::Migration)]
    }
}
