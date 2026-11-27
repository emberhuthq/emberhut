//! Everything the app's state reads, read in one transaction, so its parts agree (a message
//! stored while the state is built is either in all of them or in none).

use std::collections::BTreeMap;

use emberhut_core::message::{HistoryQuery, Message};
use emberhut_core::room::Room;
use emberhut_core::upload::StoredFile;
use emberhut_core::{MessageId, RoomId};
use sea_orm::{DatabaseConnection, TransactionTrait};

use crate::{DbError, files, messages, rooms};

/// A room's line in the switcher, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomLine {
    /// The room.
    pub room: Room,
    /// The last thing said there (events left out).
    pub last: Option<Message>,
    /// The assistant's messages there since the room was last seen.
    pub unread: u64,
}

/// What the state of one room reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Every room's line, oldest room first.
    pub rooms: Vec<RoomLine>,
    /// The room's newest page of messages, oldest first.
    pub chat: Vec<Message>,
    /// Whether older messages than the page exist.
    pub earlier: bool,
    /// The messages the page's replies quote, that still exist.
    pub quoted: Vec<Message>,
    /// The files of the page's messages, of the quoted ones and of each room's last message, by
    /// message (a message without files has no entry).
    pub files: BTreeMap<MessageId, Vec<StoredFile>>,
}

/// A page of a room's history, with what its messages show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
    /// The messages, oldest first.
    pub chat: Vec<Message>,
    /// The messages the page's replies quote, that still exist.
    pub quoted: Vec<Message>,
    /// The files of the page's messages and of the quoted ones, by message.
    pub files: BTreeMap<MessageId, Vec<StoredFile>>,
}

/// Reads snapshots.
#[derive(Clone, Debug)]
pub struct SnapshotStore {
    conn: DatabaseConnection,
}

impl SnapshotStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// The snapshot of `room` (and of every room's line), read in one transaction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn read(&self, room: RoomId) -> Result<Snapshot, DbError> {
        // A read transaction: in WAL mode it reads one version of the file throughout, and it
        // takes no write lock.
        let txn = self.conn.begin().await?;
        let mut lines = Vec::new();
        // The owner's rooms only: a family member's are theirs alone.
        let all = rooms::owners_rows(&txn).await?;
        for row in all {
            let each = rooms::from_row(row)?;
            let last = messages::last_said(&txn, each.id).await?;
            let unread = messages::count_unread(&txn, each.id, each.seen_through).await?;
            lines.push(RoomLine {
                room: each,
                last,
                unread,
            });
        }
        let chat = messages::page(&txn, room, HistoryQuery::default()).await?;
        let earlier = match chat.first() {
            Some(first) => messages::has_before(&txn, room, first.id).await?,
            None => false,
        };
        let replies: Vec<MessageId> = chat.iter().filter_map(|m| m.reply_to).collect();
        let quoted = messages::get_many(&txn, &replies).await?;
        let shown: Vec<MessageId> = chat
            .iter()
            .chain(&quoted)
            .chain(lines.iter().filter_map(|line| line.last.as_ref()))
            .map(|message| message.id)
            .collect();
        let files = files::of_messages(&txn, &shown).await?;
        txn.commit().await?;
        Ok(Snapshot {
            rooms: lines,
            chat,
            earlier,
            quoted,
            files,
        })
    }

    /// A page of `room`'s history (see [`HistoryQuery`]), with the messages its replies quote
    /// and every file they show, read in one transaction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn history(&self, room: RoomId, query: HistoryQuery) -> Result<HistoryPage, DbError> {
        let txn = self.conn.begin().await?;
        let chat = messages::page(&txn, room, query).await?;
        let replies: Vec<MessageId> = chat.iter().filter_map(|m| m.reply_to).collect();
        let quoted = messages::get_many(&txn, &replies).await?;
        let shown: Vec<MessageId> = chat.iter().chain(&quoted).map(|m| m.id).collect();
        let files = files::of_messages(&txn, &shown).await?;
        txn.commit().await?;
        Ok(HistoryPage {
            chat,
            quoted,
            files,
        })
    }
}
