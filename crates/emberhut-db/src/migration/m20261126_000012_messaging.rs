//! The owner's messages from outside the app, and the assistant's own WhatsApp number:
//!
//! - `slack`: the owner's Slack, one row per message (notifications, the owner's own
//!   messages, and what Slack's Web API gives in a workspace with the owner's token);
//! - `mail`: the owner's Gmail accounts, read only, one row per message;
//! - `voice_notes`: voice notes and audio clips, each transcribed once;
//! - `wa_chats`, `wa_msgs`, `wa_sent`: the assistant's own WhatsApp number's allow-list, the
//!   allowed chats' recent messages (context), and every message it sent (the log);
//! - `issues.last_ask` and `issue_events.answers`: the history line of a task's latest ask, and
//!   on the person's answer the ask it answered, so an OK to send as the owner counts only for
//!   the card that showed the message.
//!
//! Times are milliseconds since the epoch, as everywhere in this database.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, boolean, double_null, integer, pk_auto, string, string_null,
    text, text_null,
};

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait]
#[expect(
    elided_lifetimes_in_paths,
    reason = "async_trait matches the trait's signature, which elides SchemaManager's lifetime"
)]
impl MigrationTrait for Migration {
    #[expect(
        clippy::too_many_lines,
        reason = "one migration makes the area's six tables, each a short builder"
    )]
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Slack::Table)
                    .col(string(Slack::Id).primary_key())
                    .col(string(Slack::Workspace))
                    .col(text(Slack::Who))
                    .col(text(Slack::Body))
                    .col(big_integer(Slack::At))
                    .col(boolean(Slack::FromMe).default(false))
                    .col(text_null(Slack::Sender))
                    .col(string_null(Slack::Channel))
                    .to_owned(),
            )
            .await?;
        index(manager, "slack_at", Slack::Table, &[Slack::At]).await?;
        index(
            manager,
            "slack_conversation",
            Slack::Table,
            &[Slack::Workspace, Slack::Who, Slack::At],
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(Mail::Table)
                    .col(string(Mail::Id).primary_key())
                    .col(string(Mail::Account))
                    .col(text(Mail::Chat))
                    .col(text(Mail::Who))
                    .col(text(Mail::Sender))
                    .col(boolean(Mail::FromMe).default(false))
                    .col(text(Mail::Subject))
                    .col(text(Mail::Body))
                    .col(big_integer(Mail::At))
                    .col(big_integer(Mail::Got))
                    .col(boolean(Mail::Loud).default(true))
                    .col(string_null(Mail::Thread))
                    .to_owned(),
            )
            .await?;
        index(manager, "mail_got", Mail::Table, &[Mail::Got]).await?;
        index(manager, "mail_chat", Mail::Table, &[Mail::Chat, Mail::At]).await?;

        manager
            .create_table(
                Table::create()
                    .table(VoiceNotes::Table)
                    .col(string(VoiceNotes::Id).primary_key())
                    .col(string(VoiceNotes::Source))
                    .col(text(VoiceNotes::Ref))
                    .col(text_null(VoiceNotes::Name))
                    .col(double_null(VoiceNotes::Seconds))
                    .col(string(VoiceNotes::Status).default("new"))
                    .col(text_null(VoiceNotes::Text))
                    .col(text_null(VoiceNotes::Error))
                    .col(integer(VoiceNotes::Tries).default(0))
                    .col(big_integer_null(VoiceNotes::NextAt))
                    .col(text_null(VoiceNotes::Chat))
                    .col(text_null(VoiceNotes::ChatName))
                    .col(text_null(VoiceNotes::Sender))
                    .col(big_integer_null(VoiceNotes::MsgAt))
                    .col(text_null(VoiceNotes::Msg))
                    .col(text_null(VoiceNotes::Shown))
                    .col(big_integer(VoiceNotes::At))
                    .col(big_integer_null(VoiceNotes::DoneAt))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "voice_notes_status",
            VoiceNotes::Table,
            &[VoiceNotes::Status, VoiceNotes::NextAt],
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(WaChats::Table)
                    .col(string(WaChats::Jid).primary_key())
                    .col(string(WaChats::Kind))
                    .col(text_null(WaChats::Name))
                    .col(string(WaChats::Status).default("pending"))
                    .col(string_null(WaChats::Via))
                    .col(big_integer_null(WaChats::Task))
                    .col(big_integer_null(WaChats::Room))
                    .col(text_null(WaChats::Topics))
                    .col(string_null(WaChats::By))
                    .col(big_integer(WaChats::Added))
                    .col(big_integer(WaChats::Updated))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(WaMsgs::Table)
                    .col(string(WaMsgs::Chat))
                    .col(string(WaMsgs::Id))
                    .col(string_null(WaMsgs::Sender))
                    .col(text_null(WaMsgs::Who))
                    .col(text_null(WaMsgs::Text))
                    .col(big_integer(WaMsgs::At))
                    .col(boolean(WaMsgs::FromMe).default(false))
                    .col(boolean(WaMsgs::Asked).default(false))
                    .col(boolean(WaMsgs::Answered).default(false))
                    .primary_key(Index::create().col(WaMsgs::Chat).col(WaMsgs::Id))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "wa_msgs_at",
            WaMsgs::Table,
            &[WaMsgs::Chat, WaMsgs::At],
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table(WaSent::Table)
                    .col(pk_auto(WaSent::Id))
                    .col(string(WaSent::Chat))
                    .col(text_null(WaSent::Name))
                    .col(big_integer(WaSent::At))
                    .col(text(WaSent::Text))
                    .col(string_null(WaSent::Msg))
                    .col(string_null(WaSent::ReplyTo))
                    .col(big_integer_null(WaSent::Room))
                    .to_owned(),
            )
            .await?;
        index(manager, "wa_sent_at", WaSent::Table, &[WaSent::At]).await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Issues::Table)
                    .add_column(big_integer_null(Issues::LastAsk))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(IssueEvents::Table)
                    .add_column(big_integer_null(IssueEvents::Answers))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(IssueEvents::Table)
                    .drop_column(IssueEvents::Answers)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Issues::Table)
                    .drop_column(Issues::LastAsk)
                    .to_owned(),
            )
            .await?;
        for table in [
            WaSent::Table.into_iden(),
            WaMsgs::Table.into_iden(),
            WaChats::Table.into_iden(),
            VoiceNotes::Table.into_iden(),
            Mail::Table.into_iden(),
            Slack::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

/// An index called `name` on `table`'s `columns`.
async fn index<T: Iden + Clone + 'static, C: Iden + Clone + 'static>(
    manager: &SchemaManager<'_>,
    name: &str,
    table: T,
    columns: &[C],
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(table);
    for column in columns {
        index.col(column.clone());
    }
    manager.create_index(index).await
}

#[derive(Clone, DeriveIden)]
enum Slack {
    Table,
    Id,
    Workspace,
    Who,
    Body,
    At,
    FromMe,
    Sender,
    Channel,
}

#[derive(Clone, DeriveIden)]
enum Mail {
    Table,
    Id,
    Account,
    Chat,
    Who,
    Sender,
    FromMe,
    Subject,
    Body,
    At,
    Got,
    Loud,
    Thread,
}

#[derive(Clone, DeriveIden)]
enum VoiceNotes {
    Table,
    Id,
    Source,
    Ref,
    Name,
    Seconds,
    Status,
    Text,
    Error,
    Tries,
    NextAt,
    Chat,
    ChatName,
    Sender,
    MsgAt,
    Msg,
    Shown,
    At,
    DoneAt,
}

#[derive(Clone, DeriveIden)]
enum WaChats {
    Table,
    Jid,
    Kind,
    Name,
    Status,
    Via,
    Task,
    Room,
    Topics,
    By,
    Added,
    Updated,
}

#[derive(Clone, DeriveIden)]
enum WaMsgs {
    Table,
    Chat,
    Id,
    Sender,
    Who,
    Text,
    At,
    FromMe,
    Asked,
    Answered,
}

#[derive(Clone, DeriveIden)]
enum WaSent {
    Table,
    Id,
    Chat,
    Name,
    At,
    Text,
    Msg,
    ReplyTo,
    Room,
}

#[derive(DeriveIden)]
enum Issues {
    Table,
    LastAsk,
}

#[derive(DeriveIden)]
enum IssueEvents {
    Table,
    Answers,
}
