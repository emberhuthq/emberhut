//! Coding threads and the ones the assistant follows.

use emberhut_core::thread::{CodingThread, Ending, Followed, ThreadStatus, ending};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};

use crate::entity::{followed_thread, thread};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// What recording a thread's new look found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// The look it replaced, if the thread was known.
    pub previous: Option<CodingThread>,
    /// The thread's follow, as it was before this look, if it is followed.
    pub followed: Option<Followed>,
    /// What the new look means for its follower: set only for a followed thread that has just
    /// come to an end. An [`Ending::Ended`] thread is no longer followed.
    pub ending: Option<Ending>,
}

/// Reads and writes coding threads and their follows.
#[derive(Clone, Debug)]
pub struct ThreadStore {
    conn: DatabaseConnection,
}

impl ThreadStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// The thread `id` on `machine`, if it is known.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn get(&self, machine: &str, id: &str) -> Result<Option<CodingThread>, DbError> {
        find(&self.conn, machine, id).await
    }

    /// Stores `thread`'s new look, replacing the last one, and applies what it means to its
    /// follow, in one transaction: a followed thread that has just come to an end is no longer
    /// followed, unless it only waits for an answer.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn record(&self, thread: &CodingThread) -> Result<Recorded, DbError> {
        let txn = begin_write(&self.conn).await?;
        let previous = find(&txn, &thread.machine, &thread.id).await?;
        upsert(&txn, thread).await?;
        let followed = followed_thread::Entity::find_by_id(thread.id.clone())
            .filter(followed_thread::Column::Machine.eq(thread.machine.as_str()))
            .one(&txn)
            .await?
            .map(followed_from_row)
            .transpose()?;
        let ending = match followed {
            Some(_) => ending(previous.as_ref().map(|was| &was.status), thread),
            None => None,
        };
        if ending == Some(Ending::Ended) {
            followed_thread::Entity::delete_by_id(thread.id.clone())
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(Recorded {
            previous,
            followed,
            ending,
        })
    }

    /// Forgets the thread `id` on `machine` (its follow stays until it is unfollowed); `false`
    /// when it wasn't known.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn remove(&self, machine: &str, id: &str) -> Result<bool, DbError> {
        let removed = thread::Entity::delete_by_id((machine.to_owned(), id.to_owned()))
            .exec(&self.conn)
            .await?;
        Ok(removed.rows_affected > 0)
    }

    /// The newest `limit` top-level threads (helper agents' left out), every machine's,
    /// most recently changed first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn recent(&self, limit: u64) -> Result<Vec<CodingThread>, DbError> {
        thread::Entity::find()
            .filter(top_level())
            .order_by_desc(thread::Column::UpdatedAt)
            .order_by_asc(thread::Column::Machine)
            .order_by_asc(thread::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// Every thread on any of `machines` that still holds attention (queued, running or
    /// waiting) and is top-level, the newest `limit`, most recently changed first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn active_on(
        &self,
        machines: &[String],
        limit: u64,
    ) -> Result<Vec<CodingThread>, DbError> {
        if machines.is_empty() {
            return Ok(Vec::new());
        }
        let active = [
            ThreadStatus::Queued,
            ThreadStatus::Running,
            ThreadStatus::Waiting,
        ]
        .map(|status| status.as_str().to_owned());
        thread::Entity::find()
            .filter(top_level())
            .filter(thread::Column::Machine.is_in(machines.iter().cloned()))
            .filter(thread::Column::Status.is_in(active))
            .order_by_desc(thread::Column::UpdatedAt)
            .order_by_asc(thread::Column::Machine)
            .order_by_asc(thread::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// Every known thread, most recently changed first: helper agents' too when `helpers`, and
    /// only those in `status` when one is given.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn all(
        &self,
        helpers: bool,
        status: Option<&ThreadStatus>,
    ) -> Result<Vec<CodingThread>, DbError> {
        let mut query = thread::Entity::find();
        if !helpers {
            query = query.filter(top_level());
        }
        if let Some(status) = status {
            query = query.filter(thread::Column::Status.eq(status.as_str()));
        }
        query
            .order_by_desc(thread::Column::UpdatedAt)
            .order_by_asc(thread::Column::Machine)
            .order_by_asc(thread::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// Every known thread on `machine`, helper agents' too.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn on_machine(&self, machine: &str) -> Result<Vec<CodingThread>, DbError> {
        thread::Entity::find()
            .filter(thread::Column::Machine.eq(machine))
            .order_by_asc(thread::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// Forgets every thread on `machine` but those in `keep` (a fresh listing of that machine
    /// lists them all); how many were forgotten.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn forget_others(&self, machine: &str, keep: &[String]) -> Result<u64, DbError> {
        let removed = thread::Entity::delete_many()
            .filter(thread::Column::Machine.eq(machine))
            .filter(thread::Column::Id.is_not_in(keep.iter().cloned()))
            .exec(&self.conn)
            .await?;
        Ok(removed.rows_affected)
    }

    /// Follows `follow.thread`: its end will be news for the assistant. Following a thread
    /// again replaces its reason and time.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn follow(&self, follow: &Followed) -> Result<(), DbError> {
        followed_thread::Entity::insert(followed_thread::ActiveModel {
            thread: Set(follow.thread.clone()),
            machine: Set(follow.machine.clone()),
            why: Set(follow.why.clone()),
            at: Set(millis(follow.at)),
        })
        .on_conflict(
            OnConflict::column(followed_thread::Column::Thread)
                .update_columns([
                    followed_thread::Column::Machine,
                    followed_thread::Column::Why,
                    followed_thread::Column::At,
                ])
                .to_owned(),
        )
        .exec_without_returning(&self.conn)
        .await?;
        Ok(())
    }

    /// Stops following the thread `id`; `false` when it wasn't followed.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn unfollow(&self, id: &str) -> Result<bool, DbError> {
        let removed = followed_thread::Entity::delete_by_id(id.to_owned())
            .exec(&self.conn)
            .await?;
        Ok(removed.rows_affected > 0)
    }

    /// Every followed thread, the longest followed first, each with its latest look when one
    /// is known, read in one transaction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn followed(&self) -> Result<Vec<(Followed, Option<CodingThread>)>, DbError> {
        let txn = self.conn.begin().await?;
        let rows = followed_thread::Entity::find()
            .order_by_asc(followed_thread::Column::At)
            .order_by_asc(followed_thread::Column::Thread)
            .all(&txn)
            .await?;
        let mut all = Vec::with_capacity(rows.len());
        for row in rows {
            let follow = followed_from_row(row)?;
            let look = find(&txn, &follow.machine, &follow.thread).await?;
            all.push((follow, look));
        }
        txn.commit().await?;
        Ok(all)
    }
}

/// A top-level thread's rows: no parent (or an empty one).
fn top_level() -> Condition {
    Condition::any()
        .add(thread::Column::Parent.is_null())
        .add(thread::Column::Parent.eq(""))
}

async fn find(
    conn: &impl ConnectionTrait,
    machine: &str,
    id: &str,
) -> Result<Option<CodingThread>, DbError> {
    thread::Entity::find_by_id((machine.to_owned(), id.to_owned()))
        .one(conn)
        .await?
        .map(from_row)
        .transpose()
}

async fn upsert(conn: &impl ConnectionTrait, t: &CodingThread) -> Result<(), DbError> {
    let row = thread::ActiveModel {
        machine: Set(t.machine.clone()),
        id: Set(t.id.clone()),
        title: Set(t.title.clone()),
        project: Set(t.project.clone()),
        status: Set(t.status.as_str().to_owned()),
        updated_at: Set(millis(t.updated_at)),
        parent: Set(t.parent.clone()),
        ask: Set(t.ask.clone()),
        last_ask: Set(t.last_ask.clone()),
        last: Set(t.last.clone()),
        status_line: Set(t.status_line.clone()),
        doing: Set(t.doing.clone()),
        account: Set(t.account.clone()),
        cwd: Set(t.cwd.clone()),
        branch: Set(t.branch.clone()),
        since: Set(t.since.map(millis)),
        direct: Set(t.direct),
        model: Set(t.model.clone()),
        effort: Set(t.effort.clone()),
        question: Set(t.question.clone()),
    };
    thread::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([thread::Column::Machine, thread::Column::Id])
                .update_columns([
                    thread::Column::Title,
                    thread::Column::Project,
                    thread::Column::Status,
                    thread::Column::UpdatedAt,
                    thread::Column::Parent,
                    thread::Column::Ask,
                    thread::Column::LastAsk,
                    thread::Column::Last,
                    thread::Column::StatusLine,
                    thread::Column::Doing,
                    thread::Column::Account,
                    thread::Column::Cwd,
                    thread::Column::Branch,
                    thread::Column::Since,
                    thread::Column::Direct,
                    thread::Column::Model,
                    thread::Column::Effort,
                    thread::Column::Question,
                ])
                .to_owned(),
        )
        .exec_without_returning(conn)
        .await?;
    Ok(())
}

fn from_row(row: thread::Model) -> Result<CodingThread, DbError> {
    Ok(CodingThread {
        status: ThreadStatus::from(row.status.as_str()),
        updated_at: instant(row.updated_at)?,
        since: row.since.map(instant).transpose()?,
        machine: row.machine,
        id: row.id,
        title: row.title,
        project: row.project,
        parent: row.parent,
        ask: row.ask,
        last_ask: row.last_ask,
        last: row.last,
        status_line: row.status_line,
        doing: row.doing,
        account: row.account,
        cwd: row.cwd,
        branch: row.branch,
        direct: row.direct,
        model: row.model,
        effort: row.effort,
        question: row.question,
    })
}

fn followed_from_row(row: followed_thread::Model) -> Result<Followed, DbError> {
    Ok(Followed {
        at: instant(row.at)?,
        thread: row.thread,
        machine: row.machine,
        why: row.why,
    })
}
