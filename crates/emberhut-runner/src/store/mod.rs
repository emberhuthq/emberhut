//! The runner's own database (SQLite): its sessions, their turns, every step of their
//! transcripts, the events the server hasn't taken yet, and every file in or out.
//!
//! One file in the runner's home, `runner.db`, in the shape earlier runners wrote it, so a
//! runner replacing another carries on its sessions. A restart reads it back and carries on what
//! was running.

use std::path::Path;

use sea_orm::sea_query::{Alias, Expr, Index, Order, Table};
use sea_orm::{
    ActiveModelTrait as _, ColumnTrait as _, ConnectOptions, ConnectionTrait as _, Database,
    DatabaseConnection, DbErr, EntityTrait as _, PaginatorTrait as _, QueryFilter as _,
    QueryOrder as _, QuerySelect as _, Schema, Set,
};

pub mod event;
pub mod outbox;
pub mod session;
pub mod transfer;
pub mod turn;

/// Now, as the store writes times: RFC 3339 in UTC with milliseconds.
#[must_use]
pub fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// A store failure.
#[derive(Debug, thiserror::Error)]
#[error("the runner's database: {0}")]
pub struct StoreError(#[from] DbErr);

/// The columns of a session a change may set; `None` leaves one as it is.
#[derive(Debug, Default, Clone)]
pub struct SessionChange {
    /// Its title.
    pub title: Option<String>,
    /// Its status.
    pub status: Option<String>,
    /// Its account.
    pub account: Option<Option<String>>,
    /// The agent's session id.
    pub claude: Option<Option<String>>,
    /// Whether its transcript exists.
    pub materialized: Option<i64>,
    /// Its model.
    pub model: Option<Option<String>>,
    /// Its effort.
    pub effort: Option<Option<String>>,
    /// Its meta, as JSON.
    pub meta: Option<Option<String>>,
    /// Its last turn's number.
    pub seq: Option<i64>,
    /// When it carries on by itself.
    pub resume_at: Option<Option<String>>,
    /// Its background jobs.
    pub background: Option<i64>,
}

/// The runner's database.
#[derive(Debug, Clone)]
pub struct Store {
    db: DatabaseConnection,
}

impl Store {
    /// Opens (or makes) `runner.db` in `home`, with its tables.
    ///
    /// # Errors
    /// The file can't be opened, or its tables made.
    pub async fn open(home: &Path) -> Result<Self, StoreError> {
        Self::connect(&format!(
            "sqlite://{}?mode=rwc",
            home.join("runner.db").display()
        ))
        .await
    }

    /// A store in memory (for tests).
    ///
    /// # Errors
    /// The tables can't be made.
    pub async fn in_memory() -> Result<Self, StoreError> {
        Self::connect("sqlite::memory:").await
    }

    async fn connect(url: &str) -> Result<Self, StoreError> {
        let mut options = ConnectOptions::new(url);
        // One writer: SQLite serialises writes anyway, and one connection keeps an in-memory
        // database a single database.
        options
            .max_connections(1)
            .min_connections(1)
            .sqlx_logging(false);
        let db = Database::connect(options).await?;
        // WAL with synchronous=normal: a commit survives the process crashing and is at risk only
        // on power loss, without an fsync per transcript step.
        for pragma in [
            "PRAGMA journal_mode = WAL",
            "PRAGMA synchronous = NORMAL",
            "PRAGMA busy_timeout = 10000",
        ] {
            db.execute_unprepared(pragma).await?;
        }
        let store = Self { db };
        store.ensure_schema().await?;
        Ok(store)
    }

    /// Makes the tables that are missing, and the columns a database from an earlier runner lacks.
    async fn ensure_schema(&self) -> Result<(), DbErr> {
        let backend = self.db.get_database_backend();
        let schema = Schema::new(backend);
        let mut tables = vec![
            schema.create_table_from_entity(session::Entity),
            schema.create_table_from_entity(turn::Entity),
            schema.create_table_from_entity(event::Entity),
            schema.create_table_from_entity(outbox::Entity),
            schema.create_table_from_entity(transfer::Entity),
        ];
        for t in &mut tables {
            self.db.execute(t.if_not_exists()).await?;
        }
        let index = Index::create()
            .if_not_exists()
            .name("events_session")
            .table(event::Entity)
            .col(event::Column::Session)
            .col(event::Column::Seq)
            .to_owned();
        self.db.execute(&index).await?;
        // A session's latest step of a kind (its to-do list) without reading every step it has:
        // a long session's steps are many and large.
        let by_kind = Index::create()
            .if_not_exists()
            .name("events_kind")
            .table(event::Entity)
            .col(event::Column::Session)
            .col(event::Column::Kind)
            .col(event::Column::Seq)
            .to_owned();
        self.db.execute(&by_kind).await?;
        // Columns added after the first schema: a database made before them gets them here.
        for (column, def) in [
            ("background", "integer not null default 0"),
            ("origin", "text"),
        ] {
            let has = self
                .db
                .query_one_raw(sea_orm::Statement::from_sql_and_values(
                    backend,
                    "SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?",
                    [column.into()],
                ))
                .await?
                .is_some();
            if !has {
                let mut alter = Table::alter();
                alter.table(session::Entity).add_column(
                    sea_orm::sea_query::ColumnDef::new(Alias::new(column)).custom(Alias::new(def)),
                );
                self.db.execute(&alter).await?;
            }
        }
        Ok(())
    }

    // ── Sessions ────────────────────────────────────────────────────────────────────────────

    /// A session.
    ///
    /// # Errors
    /// The database fails.
    pub async fn session(&self, id: &str) -> Result<Option<session::Model>, StoreError> {
        Ok(session::Entity::find_by_id(id).one(&self.db).await?)
    }

    /// Adds a session.
    ///
    /// # Errors
    /// The database fails (an id taken, for one).
    pub async fn add_session(&self, s: session::Model) -> Result<(), StoreError> {
        session::ActiveModel::from(s).insert(&self.db).await?;
        Ok(())
    }

    /// Changes a session's columns and its `updated` time.
    ///
    /// # Errors
    /// The database fails.
    pub async fn change_session(&self, id: &str, c: SessionChange) -> Result<(), StoreError> {
        let mut m = session::ActiveModel {
            id: Set(id.to_owned()),
            updated: Set(Some(now())),
            ..Default::default()
        };
        if let Some(v) = c.title {
            m.title = Set(Some(v));
        }
        if let Some(v) = c.status {
            m.status = Set(v);
        }
        if let Some(v) = c.account {
            m.account = Set(v);
        }
        if let Some(v) = c.claude {
            m.claude = Set(v);
        }
        if let Some(v) = c.materialized {
            m.materialized = Set(v);
        }
        if let Some(v) = c.model {
            m.model = Set(v);
        }
        if let Some(v) = c.effort {
            m.effort = Set(v);
        }
        if let Some(v) = c.meta {
            m.meta = Set(v);
        }
        if let Some(v) = c.seq {
            m.seq = Set(v);
        }
        if let Some(v) = c.resume_at {
            m.resume_at = Set(v);
        }
        if let Some(v) = c.background {
            m.background = Set(v);
        }
        session::Entity::update(m).exec(&self.db).await?;
        Ok(())
    }

    /// Sessions, most recently changed first.
    ///
    /// # Errors
    /// The database fails.
    pub async fn sessions(&self, limit: u64) -> Result<Vec<session::Model>, StoreError> {
        Ok(session::Entity::find()
            .order_by_desc(session::Column::Updated)
            .limit(limit)
            .all(&self.db)
            .await?)
    }

    /// Sessions waiting on background jobs.
    ///
    /// # Errors
    /// The database fails.
    pub async fn sessions_with_background(&self) -> Result<Vec<session::Model>, StoreError> {
        Ok(session::Entity::find()
            .filter(session::Column::Background.gt(0))
            .all(&self.db)
            .await?)
    }

    /// Sessions due to carry on by themselves.
    ///
    /// # Errors
    /// The database fails.
    pub async fn sessions_due(&self, at: &str) -> Result<Vec<session::Model>, StoreError> {
        Ok(session::Entity::find()
            .filter(session::Column::ResumeAt.is_not_null())
            .filter(session::Column::ResumeAt.lte(at))
            .all(&self.db)
            .await?)
    }

    // ── Turns ───────────────────────────────────────────────────────────────────────────────

    /// A turn.
    ///
    /// # Errors
    /// The database fails.
    pub async fn turn(&self, id: &str) -> Result<Option<turn::Model>, StoreError> {
        Ok(turn::Entity::find_by_id(id).one(&self.db).await?)
    }

    /// Adds a turn.
    ///
    /// # Errors
    /// The database fails.
    pub async fn add_turn(&self, t: turn::Model) -> Result<(), StoreError> {
        turn::ActiveModel::from(t).insert(&self.db).await?;
        Ok(())
    }

    /// Saves a turn as it is now.
    ///
    /// # Errors
    /// The database fails.
    pub async fn save_turn(&self, t: turn::Model) -> Result<(), StoreError> {
        turn::ActiveModel::from(t)
            .reset_all()
            .update(&self.db)
            .await?;
        Ok(())
    }

    /// A session's next turn that hasn't started, oldest first.
    ///
    /// # Errors
    /// The database fails.
    pub async fn next_turn(&self, session: &str) -> Result<Option<turn::Model>, StoreError> {
        Ok(Self::waiting(session)
            .order_by_asc(turn::Column::N)
            .one(&self.db)
            .await?)
    }

    /// A session's turns that haven't started, oldest first.
    ///
    /// # Errors
    /// The database fails.
    pub async fn waiting_turns(&self, session: &str) -> Result<Vec<turn::Model>, StoreError> {
        Ok(Self::waiting(session)
            .order_by_asc(turn::Column::N)
            .all(&self.db)
            .await?)
    }

    fn waiting(session: &str) -> sea_orm::Select<turn::Entity> {
        turn::Entity::find()
            .filter(turn::Column::Session.eq(session))
            .filter(turn::Column::Started.is_null())
            .filter(turn::Column::Ended.is_null())
    }

    /// How many of a session's turns wait behind the running one.
    ///
    /// # Errors
    /// The database fails.
    pub async fn queued(&self, session: &str) -> Result<u64, StoreError> {
        Ok(Self::waiting(session).count(&self.db).await?)
    }

    /// How many turns wait, across sessions.
    ///
    /// # Errors
    /// The database fails.
    pub async fn queued_all(&self) -> Result<u64, StoreError> {
        Ok(turn::Entity::find()
            .filter(turn::Column::Started.is_null())
            .filter(turn::Column::Ended.is_null())
            .count(&self.db)
            .await?)
    }

    /// Turns that started and never ended (a restart cut them off), by session and number.
    ///
    /// # Errors
    /// The database fails.
    pub async fn cut_off_turns(&self) -> Result<Vec<turn::Model>, StoreError> {
        Ok(turn::Entity::find()
            .filter(turn::Column::Started.is_not_null())
            .filter(turn::Column::Ended.is_null())
            .order_by_asc(turn::Column::Session)
            .order_by_asc(turn::Column::N)
            .all(&self.db)
            .await?)
    }

    /// The sessions with turns that haven't started.
    ///
    /// # Errors
    /// The database fails.
    pub async fn sessions_with_waiting_turns(&self) -> Result<Vec<String>, StoreError> {
        let rows = turn::Entity::find()
            .select_only()
            .column(turn::Column::Session)
            .distinct()
            .filter(turn::Column::Started.is_null())
            .filter(turn::Column::Ended.is_null())
            .into_tuple::<String>()
            .all(&self.db)
            .await?;
        Ok(rows)
    }

    /// A session's latest turns, newest first.
    ///
    /// # Errors
    /// The database fails.
    pub async fn turns(&self, session: &str, limit: u64) -> Result<Vec<turn::Model>, StoreError> {
        Ok(turn::Entity::find()
            .filter(turn::Column::Session.eq(session))
            .order_by_desc(turn::Column::N)
            .limit(limit)
            .all(&self.db)
            .await?)
    }

    // ── Transcript steps ────────────────────────────────────────────────────────────────────

    /// Writes a step; returns it as stored.
    ///
    /// # Errors
    /// The database fails.
    pub async fn add_event(
        &self,
        session: &str,
        turn: Option<&str>,
        kind: &str,
        data: &serde_json::Value,
    ) -> Result<event::Model, StoreError> {
        let m = event::ActiveModel {
            session: Set(session.to_owned()),
            turn: Set(turn.map(str::to_owned)),
            at: Set(Some(now())),
            kind: Set(Some(kind.to_owned())),
            data: Set(Some(data.to_string())),
            ..Default::default()
        };
        Ok(m.insert(&self.db).await?)
    }

    /// A session's steps after `after`, the first `limit` of them.
    ///
    /// # Errors
    /// The database fails.
    pub async fn events_after(
        &self,
        session: &str,
        after: i64,
        limit: u64,
    ) -> Result<Vec<event::Model>, StoreError> {
        Ok(event::Entity::find()
            .filter(event::Column::Session.eq(session))
            .filter(event::Column::Seq.gt(after))
            .order_by_asc(event::Column::Seq)
            .limit(limit)
            .all(&self.db)
            .await?)
    }

    /// A session's newest `limit` steps, oldest first.
    ///
    /// # Errors
    /// The database fails.
    pub async fn events_newest(
        &self,
        session: &str,
        limit: u64,
    ) -> Result<Vec<event::Model>, StoreError> {
        let mut rows = event::Entity::find()
            .filter(event::Column::Session.eq(session))
            .order_by(event::Column::Seq, Order::Desc)
            .limit(limit)
            .all(&self.db)
            .await?;
        rows.reverse();
        Ok(rows)
    }

    /// Whether a session has steps before `seq`.
    ///
    /// # Errors
    /// The database fails.
    pub async fn has_events_before(&self, session: &str, seq: i64) -> Result<bool, StoreError> {
        Ok(event::Entity::find()
            .filter(event::Column::Session.eq(session))
            .filter(event::Column::Seq.lt(seq))
            .count(&self.db)
            .await?
            > 0)
    }

    /// A session's latest step of a kind.
    ///
    /// # Errors
    /// The database fails.
    pub async fn last_event_of(
        &self,
        session: &str,
        kind: &str,
    ) -> Result<Option<event::Model>, StoreError> {
        Ok(event::Entity::find()
            .filter(event::Column::Session.eq(session))
            .filter(event::Column::Kind.eq(kind))
            .order_by_desc(event::Column::Seq)
            .one(&self.db)
            .await?)
    }

    // ── Outbox ──────────────────────────────────────────────────────────────────────────────

    /// Queues an event for the server; a newer `activity` replaces the session's older ones not yet sent.
    ///
    /// # Errors
    /// The database fails.
    pub async fn push_outbox(
        &self,
        session: &str,
        kind: &str,
        body: &str,
        replaces_older: bool,
    ) -> Result<(), StoreError> {
        if replaces_older {
            outbox::Entity::delete_many()
                .filter(outbox::Column::Session.eq(session))
                .filter(outbox::Column::Kind.eq(kind))
                .exec(&self.db)
                .await?;
        }
        outbox::ActiveModel {
            session: Set(Some(session.to_owned())),
            kind: Set(Some(kind.to_owned())),
            body: Set(Some(body.to_owned())),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// The oldest event waiting for the server.
    ///
    /// # Errors
    /// The database fails.
    pub async fn first_outbox(&self) -> Result<Option<outbox::Model>, StoreError> {
        Ok(outbox::Entity::find()
            .order_by_asc(outbox::Column::Id)
            .one(&self.db)
            .await?)
    }

    /// Drops an event the server took.
    ///
    /// # Errors
    /// The database fails.
    pub async fn drop_outbox(&self, id: i64) -> Result<(), StoreError> {
        outbox::Entity::delete_by_id(id).exec(&self.db).await?;
        Ok(())
    }

    /// How many events wait for the server.
    ///
    /// # Errors
    /// The database fails.
    pub async fn outbox_len(&self) -> Result<u64, StoreError> {
        Ok(outbox::Entity::find().count(&self.db).await?)
    }

    // ── Transfers ───────────────────────────────────────────────────────────────────────────

    /// Records a transfer or a refusal.
    ///
    /// # Errors
    /// The database fails.
    pub async fn record_transfer(&self, t: transfer::Model) -> Result<(), StoreError> {
        let mut m = transfer::ActiveModel::from(t);
        m.id = sea_orm::NotSet;
        m.insert(&self.db).await?;
        Ok(())
    }

    /// The files that arrived for a session since it was last told, marked told.
    ///
    /// # Errors
    /// The database fails.
    pub async fn take_arrived(&self, session: &str) -> Result<Vec<transfer::Model>, StoreError> {
        let rows = transfer::Entity::find()
            .filter(transfer::Column::Session.eq(session))
            .filter(transfer::Column::Dir.eq("in"))
            .filter(transfer::Column::Ok.eq(1))
            .filter(transfer::Column::Told.eq(0))
            .order_by_asc(transfer::Column::Id)
            .all(&self.db)
            .await?;
        if !rows.is_empty() {
            transfer::Entity::update_many()
                .col_expr(transfer::Column::Told, Expr::value(1))
                .filter(transfer::Column::Id.is_in(rows.iter().map(|r| r.id)))
                .exec(&self.db)
                .await?;
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_session(id: &str) -> session::Model {
        session::Model {
            id: id.to_owned(),
            title: Some("T".to_owned()),
            cwd: "/w".to_owned(),
            project: None,
            model: None,
            effort: None,
            account: Some("a".to_owned()),
            claude: None,
            materialized: 0,
            status: "queued".to_owned(),
            worktree: None,
            branch: None,
            meta: None,
            seq: 0,
            resume_at: None,
            created: Some(now()),
            updated: Some(now()),
            background: 0,
            origin: Some("/w".to_owned()),
        }
    }

    #[tokio::test]
    async fn sessions_turns_steps_and_the_outbox_keep_their_order() {
        let s = Store::in_memory().await.unwrap();
        s.add_session(a_session("r-00000001")).await.unwrap();
        s.change_session(
            "r-00000001",
            SessionChange {
                status: Some("running".to_owned()),
                seq: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            s.session("r-00000001").await.unwrap().unwrap().status,
            "running"
        );
        for n in 1..=2 {
            s.add_turn(turn::Model {
                id: format!("r-00000001:{n}"),
                session: "r-00000001".to_owned(),
                n,
                prompt: Some(format!("p{n}")),
                by: Some("owner".to_owned()),
                requested: Some(now()),
                started: None,
                ended: None,
                outcome: None,
                last: None,
                accounts: None,
            })
            .await
            .unwrap();
        }
        assert_eq!(s.queued("r-00000001").await.unwrap(), 2);
        assert_eq!(s.next_turn("r-00000001").await.unwrap().unwrap().n, 1);
        for k in ["user", "text", "end"] {
            s.add_event(
                "r-00000001",
                Some("r-00000001:1"),
                k,
                &serde_json::json!({"text": k}),
            )
            .await
            .unwrap();
        }
        let newest = s.events_newest("r-00000001", 2).await.unwrap();
        assert_eq!(
            newest
                .iter()
                .map(|e| e.kind.clone().unwrap())
                .collect::<Vec<_>>(),
            ["text", "end"]
        );
        assert!(
            s.has_events_before("r-00000001", newest[0].seq)
                .await
                .unwrap()
        );
        s.push_outbox("r-00000001", "activity", "{}", true)
            .await
            .unwrap();
        s.push_outbox("r-00000001", "activity", "{\"n\":2}", true)
            .await
            .unwrap();
        assert_eq!(
            s.outbox_len().await.unwrap(),
            1,
            "a newer activity replaces the older"
        );
    }

    #[tokio::test]
    async fn a_sessions_latest_step_of_a_kind_is_found_by_its_index() {
        let s = Store::in_memory().await.unwrap();
        let plan = s
            .db
            .query_all_raw(sea_orm::Statement::from_string(
                s.db.get_database_backend(),
                "explain query plan select * from events where session = 'r-1' and kind = 'todos' order by seq desc limit 1",
            ))
            .await
            .unwrap();
        let words: Vec<String> = plan
            .iter()
            .filter_map(|r| r.try_get_by_index::<String>(3).ok())
            .collect();
        assert!(words.iter().any(|w| w.contains("events_kind")), "{words:?}");
    }

    #[tokio::test]
    async fn a_database_without_the_newer_columns_gets_them() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = Database::connect(format!(
                "sqlite://{}?mode=rwc",
                dir.path().join("runner.db").display()
            ))
            .await
            .unwrap();
            db.execute_unprepared(
                "create table sessions (id text primary key, title text, cwd text not null, project text, model text, effort text,
                 account text, claude text, materialized integer not null default 0, status text not null default 'queued', worktree text,
                 branch text, meta text, seq integer not null default 0, resume_at text, created text, updated text)",
            )
            .await
            .unwrap();
        }
        let s = Store::open(dir.path()).await.unwrap();
        s.add_session(a_session("r-00000002")).await.unwrap();
        assert_eq!(
            s.session("r-00000002")
                .await
                .unwrap()
                .unwrap()
                .origin
                .as_deref(),
            Some("/w")
        );
    }
}
