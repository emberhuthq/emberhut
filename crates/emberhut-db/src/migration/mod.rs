//! The schema's history: every migration, in order. [`Database::connect`](crate::Database::connect)
//! applies the ones a database lacks.
//!
//! A migration is never edited once it is on `main`; a change to the schema is a new one.
//! Migrations name tables and columns with their own identifiers rather than the entities',
//! so a later change to an entity cannot rewrite history.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::{MigrationTrait, MigratorTrait};

mod m20261125_000001_create_tables;
mod m20261126_000001_rooms_seen_through;
mod m20261126_000002_settings;
mod m20261126_000003_files;
mod m20261126_000004_push_and_secret_links;
mod m20261126_000005_builds;
mod m20261126_000006_threads;
mod m20261126_000007_tasks;
mod m20261126_000008_agent;
mod m20261126_000009_memory;
mod m20261126_000010_runners;
mod m20261126_000011_calls_calendar_health;
mod m20261126_000012_messaging;
mod m20261126_000013_workers;
mod m20261126_000014_chrome;
mod m20261126_000015_parties;
mod m20261126_000016_family;
mod m20261126_000017_dark;
mod m20261126_000018_import_archive;
mod m20261126_000019_import_links;
mod m20261127_000001_agents_and_approvals;
mod m20261127_000002_agent_worker_shell;
mod m20261127_000003_correction_lesson_to_owner;
mod m20261128_000001_task_plans;
mod m20261128_000002_paired_runners;
mod m20261128_000002_session_tasks;

/// Every migration of the server's database.
#[derive(Debug)]
pub(crate) struct Migrator;

#[async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261125_000001_create_tables::Migration),
            Box::new(m20261126_000001_rooms_seen_through::Migration),
            Box::new(m20261126_000002_settings::Migration),
            Box::new(m20261126_000003_files::Migration),
            Box::new(m20261126_000004_push_and_secret_links::Migration),
            Box::new(m20261126_000005_builds::Migration),
            Box::new(m20261126_000006_threads::Migration),
            Box::new(m20261126_000007_tasks::Migration),
            Box::new(m20261126_000008_agent::Migration),
            Box::new(m20261126_000009_memory::Migration),
            Box::new(m20261126_000010_runners::Migration),
            Box::new(m20261126_000011_calls_calendar_health::Migration),
            Box::new(m20261126_000012_messaging::Migration),
            Box::new(m20261126_000013_workers::Migration),
            Box::new(m20261126_000014_chrome::Migration),
            Box::new(m20261126_000015_parties::Migration),
            Box::new(m20261126_000016_family::Migration),
            Box::new(m20261126_000018_import_archive::Migration),
            Box::new(m20261126_000019_import_links::Migration),
            Box::new(m20261127_000001_agents_and_approvals::Migration),
            Box::new(m20261127_000002_agent_worker_shell::Migration),
            Box::new(m20261127_000003_correction_lesson_to_owner::Migration),
            Box::new(m20261128_000001_task_plans::Migration),
            Box::new(m20261128_000002_session_tasks::Migration),
            Box::new(m20261128_000002_paired_runners::Migration),
        ]
    }
}

/// The off-the-record room's own database: its one table, in a file of its own.
#[derive(Debug)]
pub(crate) struct DarkMigrator;

#[async_trait]
impl MigratorTrait for DarkMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20261126_000017_dark::Migration)]
    }
}

#[cfg(test)]
mod tests {
    use sea_orm_migration::prelude::*;

    use super::*;

    /// The first migration only, as a database made before the second.
    struct FirstOnly;

    #[async_trait]
    impl MigratorTrait for FirstOnly {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            vec![Box::new(m20261125_000001_create_tables::Migration)]
        }
    }

    #[derive(DeriveIden)]
    enum Rooms {
        Table,
        Id,
        SeenAt,
    }

    #[derive(DeriveIden)]
    enum Messages {
        Table,
        RoomId,
        Author,
        Text,
        CreatedAt,
    }

    #[tokio::test]
    async fn the_upgrade_keeps_what_was_seen_seen() {
        let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        FirstOnly::up(&conn, None).await.unwrap();
        let manager = SchemaManager::new(&conn);
        // Three messages in Main at 10, 20 and 30 ms; Main seen at 25 ms.
        for at in [10_i64, 20, 30] {
            manager
                .exec_stmt(
                    Query::insert()
                        .into_table(Messages::Table)
                        .columns([
                            Messages::RoomId,
                            Messages::Author,
                            Messages::Text,
                            Messages::CreatedAt,
                        ])
                        .values([1.into(), "assistant".into(), "hi".into(), at.into()])
                        .unwrap()
                        .to_owned(),
                )
                .await
                .unwrap();
        }
        manager
            .exec_stmt(
                Query::update()
                    .table(Rooms::Table)
                    .value(Rooms::SeenAt, 25_i64)
                    .and_where(Expr::col(Rooms::Id).eq(1))
                    .to_owned(),
            )
            .await
            .unwrap();

        Migrator::up(&conn, None).await.unwrap();
        let seen_through = <crate::entity::room::Entity as sea_orm::EntityTrait>::find_by_id(1_i64)
            .one(&conn)
            .await
            .unwrap()
            .unwrap()
            .seen_through;
        assert_eq!(
            seen_through,
            Some(2),
            "the newest message at or before 25 ms"
        );
    }

    /// Every migration before the paired runners', as a database made before them.
    struct BeforePairedRunners;

    #[async_trait]
    impl MigratorTrait for BeforePairedRunners {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            Migrator::migrations()
                .into_iter()
                .take_while(|migration| migration.name() != "m20261128_000002_paired_runners")
                .collect()
        }
    }

    #[derive(DeriveIden)]
    enum PairingCodes {
        Table,
        CodeHash,
        CreatedAt,
        ExpiresAt,
    }

    #[tokio::test]
    async fn the_upgrade_adds_the_paired_runners_and_keeps_the_device_codes() {
        use sea_orm::{EntityTrait as _, PaginatorTrait as _};

        let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        BeforePairedRunners::up(&conn, None).await.unwrap();
        assert_eq!(
            Migrator::get_pending_migrations(&conn).await.unwrap().len(),
            1
        );
        SchemaManager::new(&conn)
            .exec_stmt(
                Query::insert()
                    .into_table(PairingCodes::Table)
                    .columns([
                        PairingCodes::CodeHash,
                        PairingCodes::CreatedAt,
                        PairingCodes::ExpiresAt,
                    ])
                    .values([vec![7_u8; 32].into(), 10_i64.into(), 20_i64.into()])
                    .unwrap()
                    .to_owned(),
            )
            .await
            .unwrap();

        Migrator::up(&conn, None).await.unwrap();
        assert_eq!(
            crate::entity::pairing_code::Entity::find()
                .count(&conn)
                .await
                .unwrap(),
            1,
            "a device's code stays"
        );
        assert_eq!(
            crate::entity::paired_runner::Entity::find()
                .count(&conn)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            crate::entity::runner_code::Entity::find()
                .count(&conn)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn every_migration_goes_down_and_up_again() {
        let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&conn, None).await.unwrap();
        assert!(
            Migrator::get_pending_migrations(&conn)
                .await
                .unwrap()
                .is_empty()
        );
        Migrator::down(&conn, None).await.unwrap();
        assert_eq!(
            Migrator::get_pending_migrations(&conn).await.unwrap().len(),
            Migrator::migrations().len()
        );
        Migrator::up(&conn, None).await.unwrap();
    }
}
