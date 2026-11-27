//! A room's unread messages, counted by id: `rooms.seen_through`, the newest message the person
//! had seen. Ids only grow, so this is exact where comparing times is not (a reply stored in
//! the same millisecond as a `seen`, or a clock that steps back).
//!
//! A room already seen gets the newest of its messages at or before its `seen_at`, so what
//! was read before the upgrade stays read.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::big_integer_null;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait]
#[expect(
    elided_lifetimes_in_paths,
    reason = "async_trait matches the trait's signature, which elides SchemaManager's lifetime"
)]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Rooms::Table)
                    .add_column(big_integer_null(Rooms::SeenThrough))
                    .to_owned(),
            )
            .await?;
        let newest_seen = Query::select()
            .expr(Expr::col((Messages::Table, Messages::Id)).max())
            .from(Messages::Table)
            .and_where(
                Expr::col((Messages::Table, Messages::RoomId)).equals((Rooms::Table, Rooms::Id)),
            )
            .and_where(
                Expr::col((Messages::Table, Messages::CreatedAt))
                    .lte(Expr::col((Rooms::Table, Rooms::SeenAt))),
            )
            .to_owned();
        manager
            .exec_stmt(
                Query::update()
                    .table(Rooms::Table)
                    .value(
                        Rooms::SeenThrough,
                        Expr::SubQuery(None, Box::new(newest_seen.into())),
                    )
                    .and_where(Expr::col((Rooms::Table, Rooms::SeenAt)).is_not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Rooms::Table)
                    .drop_column(Rooms::SeenThrough)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Rooms {
    Table,
    Id,
    SeenAt,
    SeenThrough,
}

#[derive(DeriveIden)]
enum Messages {
    Table,
    Id,
    RoomId,
    CreatedAt,
}
