//! The typed rows an import maps into emberhut's tables, and their reading back for the
//! back-mapping check.
//!
//! Each record is an image of one row of an emberhut table, column for column: ids as the source
//! gave them, instants as milliseconds since the epoch (as the column stores them), days as
//! `YYYY-MM-DD` text. The importer builds them; this module is the only code that knows which
//! table and columns they are.

use emberhut_core::message::Author;
use emberhut_core::upload::FileKind;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::NotSet, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};

use super::ImportStore;
use crate::DbError;
use crate::entity::{
    area, artifact, call, call_hint, call_line, correction, doc, doc_vector, file, health_day,
    issue, issue_event, link, message, message_file, message_ref, note, project, room, run,
    screentime_day, setting, voice_note,
};

/// Rows per insert, under SQLite's limit on a statement's values for the widest table.
const BATCH: usize = 200;

/// Keys per read-back query.
const KEYS: usize = 500;

/// A record type whose fields are its table's columns, inserted as they are and read back by
/// its key column: the struct, its conversion from the entity's row, and the two store methods.
/// The columns listed after `fresh` are the server's own, which no archive has: an imported
/// row leaves them to their default (NULL), and replacing one leaves them as they are.
macro_rules! imported {
    (
        $(#[$doc:meta])*
        $name:ident in $entity:ident,
        insert $insert:ident,
        read $read:ident by $key_field:ident ($key_column:ident: $key_ty:ty) {
            $( $(#[$field_doc:meta])* $field:ident: $ty:ty, )*
        }
        $( fresh { $( $fresh:ident ),* $(,)? } )?
    ) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name {
            $( $(#[$field_doc])* pub $field: $ty, )*
        }

        impl From<$entity::Model> for $name {
            fn from(model: $entity::Model) -> Self {
                Self { $( $field: model.$field, )* }
            }
        }

        impl From<&$name> for $entity::ActiveModel {
            fn from(row: &$name) -> Self {
                Self {
                    $( $field: Set(row.$field.clone()), )*
                    $( $( $fresh: NotSet, )* )?
                }
            }
        }

        impl ImportStore {
            #[doc = concat!("Inserts imported [`", stringify!($name), "`]s as they are.")]
            ///
            /// # Errors
            ///
            /// [`DbError::Query`].
            pub async fn $insert(&self, rows: &[$name]) -> Result<(), DbError> {
                for chunk in rows.chunks(BATCH) {
                    let models = chunk.iter().map($entity::ActiveModel::from);
                    $entity::Entity::insert_many(models).exec(&self.txn).await?;
                }
                Ok(())
            }

            #[doc = concat!("The [`", stringify!($name), "`]s whose key is one of `keys`.")]
            ///
            /// # Errors
            ///
            /// [`DbError::Query`].
            pub async fn $read(&self, keys: &[$key_ty]) -> Result<Vec<$name>, DbError> {
                let mut found = Vec::with_capacity(keys.len());
                for chunk in keys.chunks(KEYS) {
                    found.extend(
                        $entity::Entity::find()
                            .filter($entity::Column::$key_column.is_in(chunk.iter().cloned()))
                            .all(&self.txn)
                            .await?
                            .into_iter()
                            .map($name::from),
                    );
                }
                Ok(found)
            }
        }
    };
}

imported! {
    /// A project (`projects`).
    ImportedProject in project,
    insert insert_projects,
    read projects_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its name.
        name: String,
        /// Its goal's id.
        goal: Option<i64>,
        /// Its priority.
        priority: i32,
        /// Its kind, in the source's spelling.
        kind: String,
        /// Its deadline, a day.
        deadline: Option<String>,
        /// Whether the deadline is hard.
        hard: bool,
        /// Its status, in the source's spelling.
        status: String,
        /// The day it is paused until.
        paused_until: Option<String>,
        /// When it was made, in milliseconds.
        created: i64,
        /// When it last changed, in milliseconds.
        updated: i64,
    }
}

imported! {
    /// An area of a project (`areas`).
    ImportedArea in area,
    insert insert_areas,
    read areas_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its project.
        pid: i64,
        /// Its name.
        name: String,
        /// Its status.
        status: String,
        /// Its goal, in words.
        goal: Option<String>,
        /// What it is about.
        about: Option<String>,
        /// When it was made, in milliseconds.
        created: i64,
        /// When it last changed, in milliseconds.
        updated: i64,
    }
}

imported! {
    /// A task (`issues`).
    ImportedTask in issue,
    insert insert_tasks,
    read tasks_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its title.
        title: String,
        /// Its brief.
        body: Option<String>,
        /// Whose it is.
        owner: String,
        /// Its status, in the source's spelling.
        status: String,
        /// Its goal's id.
        goal: Option<i64>,
        /// Its project, by name.
        project: Option<String>,
        /// Its project, by id.
        pid: Option<i64>,
        /// Whether its date is hard.
        hard: bool,
        /// Its size.
        size: Option<String>,
        /// Its priority.
        priority: i32,
        /// Its due day.
        due: Option<String>,
        /// Whether it is for today.
        today: bool,
        /// When it is parked until: a day or an instant, as given.
        parked_until: Option<String>,
        /// The day a decision is needed by.
        decide_by: Option<String>,
        /// Its ask, as JSON text.
        decision: Option<String>,
        /// The history line of its latest ask.
        last_ask: Option<i64>,
        /// What it asks for.
        action: Option<String>,
        /// When it was made, in milliseconds.
        created: i64,
        /// When it last changed, in milliseconds.
        updated: i64,
        /// When it was done, in milliseconds.
        done_at: Option<i64>,
        /// Its newest history line.
        last_note: Option<String>,
        /// Its parent task.
        parent: Option<i64>,
        /// Its progress.
        progress: Option<i64>,
        /// Its progress's total.
        total: Option<i64>,
        /// What its progress counts.
        count_unit: Option<String>,
        /// Whether it is tracked.
        tracked: bool,
        /// When it should be done, as given.
        eta: Option<String>,
        /// Whether it runs as an ultra run.
        ultra: bool,
        /// Its ultra run's lanes.
        ultra_parallel: Option<i32>,
        /// Its ultra run's budget.
        ultra_budget: Option<i64>,
        /// Its model.
        model: Option<String>,
        /// Its area.
        area: Option<i64>,
        /// Its kind.
        kind: Option<String>,
        /// Where its work landed.
        landed: Option<String>,
        /// The task that fixed it.
        fixed_by: Option<i64>,
        /// Its worker's runner.
        runner: Option<String>,
        /// Its worker's next step.
        next: Option<String>,
        /// Its worker's unit.
        unit: Option<String>,
        /// Its worker's session.
        session: Option<String>,
        /// Who asked for it, as given.
        requested: Option<String>,
        /// Its room.
        room: Option<i64>,
        /// Its ultra run.
        ultra_run: Option<String>,
        /// Its worker's turn file.
        turn_file: Option<String>,
        /// Who else asked for it, as JSON text.
        requested_by: Option<String>,
    }
    fresh { plan_updated, plan_last_step, unplanned_turn }
}

imported! {
    /// A line of a task's history (`issue_events`).
    ImportedTaskEvent in issue_event,
    insert insert_task_events,
    read task_events_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its task.
        issue: i64,
        /// When, in milliseconds.
        at: i64,
        /// Who wrote it.
        who: String,
        /// What it says.
        text: String,
        /// The ask it answers.
        answers: Option<i64>,
    }
}

imported! {
    /// Something a task produced (`artifacts`).
    ImportedArtifact in artifact,
    insert insert_artifacts,
    read artifacts_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its task.
        issue: i64,
        /// A path or a URL.
        reference: String,
        /// Its kind.
        kind: String,
        /// Its caption.
        caption: Option<String>,
        /// When, in milliseconds.
        at: i64,
        /// Its data, as JSON text.
        data: Option<String>,
    }
}

imported! {
    /// What moves a task (`links`).
    ImportedLink in link,
    insert insert_links,
    read links_by_task by issue (Issue: i64) {
        /// Its task.
        issue: i64,
        /// Its kind.
        kind: String,
        /// What it links.
        reference: String,
        /// The machine it is on.
        machine: Option<String>,
    }
}

imported! {
    /// A turn of work on a task (`runs`).
    ImportedRun in run,
    insert insert_runs,
    read runs_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its task.
        task: Option<i64>,
        /// The machine it ran on.
        machine: String,
        /// The thread or session it ran in.
        thread: String,
        /// The thread's turn.
        turn: Option<String>,
        /// Its kind, in the source's spelling.
        kind: String,
        /// What it was asked.
        asked: Option<String>,
        /// When it was asked for, in milliseconds.
        requested: Option<i64>,
        /// When it started, in milliseconds.
        started: Option<i64>,
        /// When it ended, in milliseconds.
        ended: Option<i64>,
        /// How it ended, in the source's spelling.
        outcome: Option<String>,
        /// Its last words.
        last: Option<String>,
        /// Its status line.
        status_line: Option<String>,
        /// Whether its task was inferred.
        inferred: bool,
        /// Whether its result was applied to the task.
        applied: bool,
    }
}

imported! {
    /// A setting (`settings`): its key and its JSON value.
    ImportedSetting in setting,
    insert insert_settings,
    read settings_by_key by key (Key: String) {
        /// Its key.
        key: String,
        /// Its value, as JSON text.
        value: String,
    }
}

imported! {
    /// A note (`notes`).
    ImportedNote in note,
    insert insert_notes,
    read notes_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its unique id.
        uid: String,
        /// Who wrote it, in the source's spelling.
        who: String,
        /// Its day.
        day: String,
        /// When, in milliseconds.
        at: i64,
        /// What it says.
        text: String,
        /// When it was made, in milliseconds.
        created: i64,
        /// When it last changed, in milliseconds.
        updated: i64,
    }
}

imported! {
    /// A correction and its lesson's trial (`corrections`).
    ImportedCorrection in correction,
    insert insert_corrections,
    read corrections_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// When, in milliseconds.
        at: i64,
        /// What was said.
        said: String,
        /// What was wrong.
        wrong: String,
        /// The lesson.
        lesson: Option<String>,
        /// Who logged it, in the source's spelling.
        source: String,
        /// The trial's state.
        trial: Option<String>,
        /// Who ended the trial.
        trial_by: Option<String>,
        /// When the trial ended, in milliseconds.
        trial_at: Option<i64>,
        /// The standing rule it became.
        rule: Option<i64>,
        /// The lesson as one short line to the owner.
        lesson_to_owner: Option<String>,
        /// What the nightly review made of a noticed candidate.
        triaged: Option<String>,
    }
}

imported! {
    /// A piece of text recall searches (`docs`).
    ImportedDoc in doc,
    insert insert_docs,
    read docs_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its kind.
        source: String,
        /// Where it comes from.
        reference: String,
        /// When, in milliseconds.
        at: Option<i64>,
        /// Its title.
        title: String,
        /// Its text.
        text: String,
        /// Where it was said.
        place: String,
    }
}

imported! {
    /// A piece's embedding vector (`vecs`): little-endian `f32`s.
    ImportedVector in doc_vector,
    insert insert_vectors,
    read vectors_by_id by id (Id: i64) {
        /// Its piece.
        id: i64,
        /// The vector's bytes.
        v: Vec<u8>,
    }
}

imported! {
    /// A recorded or shared call (`calls`).
    ImportedCall in call,
    insert insert_calls,
    read calls_by_id by id (Id: String) {
        /// Its id.
        id: String,
        /// Its title.
        title: String,
        /// Its task.
        task: Option<i64>,
        /// Its room.
        room_id: i64,
        /// Its status.
        status: String,
        /// Its tracks, as JSON text.
        tracks: String,
        /// Where it came from.
        source: Option<String>,
        /// Its transcript's path.
        transcript: Option<String>,
        /// What went wrong.
        error: Option<String>,
        /// How long it was.
        seconds: Option<f64>,
        /// When, in milliseconds.
        at: i64,
        /// When it was done, in milliseconds.
        done_at: Option<i64>,
        /// Whether its audio is gone.
        audio_gone: bool,
    }
}

imported! {
    /// A live transcript line of a call (`call_lines`).
    ImportedCallLine in call_line,
    insert insert_call_lines,
    read call_lines_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its call.
        call_id: String,
        /// Its track.
        track: String,
        /// Its time in the call, in seconds.
        t: f64,
        /// What was said.
        text: String,
        /// Its language.
        lang: Option<String>,
        /// Whether the transcriber was sure.
        sure: bool,
        /// When, in milliseconds.
        at: i64,
    }
}

imported! {
    /// A hint given during a call (`call_hints`).
    ImportedCallHint in call_hint,
    insert insert_call_hints,
    read call_hints_by_id by id (Id: i64) {
        /// Its id.
        id: i64,
        /// Its call.
        call_id: String,
        /// Its time in the call, in seconds.
        t: Option<f64>,
        /// The hint.
        text: String,
        /// When, in milliseconds.
        at: i64,
    }
}

imported! {
    /// A voice note and what it said (`voice_notes`).
    ImportedVoiceNote in voice_note,
    insert insert_voice_notes,
    read voice_notes_by_id by id (Id: String) {
        /// Its id.
        id: String,
        /// Where it came from.
        source: String,
        /// Its reference there.
        reference: String,
        /// Its name.
        name: Option<String>,
        /// How long it is.
        seconds: Option<f64>,
        /// Its status.
        status: String,
        /// What it said.
        text: Option<String>,
        /// What went wrong.
        error: Option<String>,
        /// How often it was tried.
        tries: i32,
        /// When it is tried next, in milliseconds.
        next_at: Option<i64>,
        /// Its chat.
        chat: Option<String>,
        /// Its chat's name.
        chat_name: Option<String>,
        /// Who sent it.
        sender: Option<String>,
        /// When it was sent, in milliseconds.
        msg_at: Option<i64>,
        /// Its message.
        msg: Option<String>,
        /// Where it was shown.
        shown: Option<String>,
        /// When it came, in milliseconds.
        at: i64,
        /// When it was done, in milliseconds.
        done_at: Option<i64>,
    }
}

imported! {
    /// A day of health data (`health`).
    ImportedHealthDay in health_day,
    insert insert_health_days,
    read health_days_by_date by date (Date: String) {
        /// The day.
        date: String,
        /// Its data, as JSON text.
        data: String,
        /// When it came, in milliseconds.
        at: i64,
    }
}

imported! {
    /// A day of a device's screen time (`screentime`).
    ImportedScreenTime in screentime_day,
    insert insert_screen_time,
    read screen_time_by_date by date (Date: String) {
        /// The day.
        date: String,
        /// The device.
        device: String,
        /// Its data, as JSON text.
        data: String,
        /// When it came, in milliseconds.
        at: i64,
    }
}

imported! {
    /// A message's chips (`message_refs`): its refs, as JSON text.
    ImportedMessageRefs in message_ref,
    insert insert_message_refs,
    read message_refs_by_message by message_id (MessageId: i64) {
        /// The message.
        message_id: i64,
        /// Its refs, as JSON text.
        refs: String,
    }
}

imported! {
    /// One of a message's files, in order (`message_files`).
    ImportedMessageFile in message_file,
    insert insert_message_files,
    read message_files_by_message by message_id (MessageId: i64) {
        /// The message.
        message_id: i64,
        /// Its place among the message's files.
        position: i32,
        /// The file.
        file_id: i64,
    }
}

/// A room (`rooms`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedRoom {
    /// Its id.
    pub id: i64,
    /// Its name.
    pub name: String,
    /// When it was made, in milliseconds.
    pub created_at: i64,
    /// When it was last seen, in milliseconds.
    pub seen_at: Option<i64>,
    /// The newest message seen.
    pub seen_through: Option<i64>,
    /// The project whose room it is.
    pub project_id: Option<i64>,
}

/// A message (`messages`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedMessage {
    /// Its id.
    pub id: i64,
    /// Its room.
    pub room_id: i64,
    /// Who said it.
    pub author: Author,
    /// What was said.
    pub text: String,
    /// When, in milliseconds.
    pub created_at: i64,
    /// The message it replies to.
    pub reply_to_id: Option<i64>,
}

/// A stored file of the chat (`files`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedFile {
    /// Its id.
    pub id: i64,
    /// Its path in the uploads folder.
    pub stored_name: String,
    /// Its name as shown.
    pub name: String,
    /// Its kind.
    pub kind: FileKind,
    /// Its size.
    pub size: i64,
    /// When it was stored, in milliseconds.
    pub created_at: i64,
}

const fn author_of(author: Author) -> message::Author {
    match author {
        Author::User => message::Author::User,
        Author::Assistant => message::Author::Assistant,
        Author::Event => message::Author::Event,
    }
}

const fn author_from(author: message::Author) -> Author {
    match author {
        message::Author::User => Author::User,
        message::Author::Assistant => Author::Assistant,
        message::Author::Event => Author::Event,
    }
}

const fn kind_of(kind: FileKind) -> file::Kind {
    match kind {
        FileKind::Image => file::Kind::Image,
        FileKind::Video => file::Kind::Video,
        FileKind::File => file::Kind::File,
    }
}

const fn kind_from(kind: file::Kind) -> FileKind {
    match kind {
        file::Kind::Image => FileKind::Image,
        file::Kind::Video => FileKind::Video,
        file::Kind::File => FileKind::File,
    }
}

impl ImportStore {
    /// Inserts imported rooms, in place of any with the same id (Main is there from the start).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn insert_rooms(&self, rows: &[ImportedRoom]) -> Result<(), DbError> {
        for chunk in rows.chunks(BATCH) {
            let models = chunk.iter().map(|row| room::ActiveModel {
                id: Set(row.id),
                name: Set(row.name.clone()),
                created_at: Set(row.created_at),
                seen_at: Set(row.seen_at),
                seen_through: Set(row.seen_through),
                project_id: Set(row.project_id),
            });
            room::Entity::insert_many(models)
                .on_conflict(
                    OnConflict::column(room::Column::Id)
                        .update_columns([
                            room::Column::Name,
                            room::Column::CreatedAt,
                            room::Column::SeenAt,
                            room::Column::SeenThrough,
                            room::Column::ProjectId,
                        ])
                        .to_owned(),
                )
                .exec(&self.txn)
                .await?;
        }
        Ok(())
    }

    /// The rooms whose id is one of `ids`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn rooms_by_id(&self, ids: &[i64]) -> Result<Vec<ImportedRoom>, DbError> {
        let mut found = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(KEYS) {
            found.extend(
                room::Entity::find()
                    .filter(room::Column::Id.is_in(chunk.iter().copied()))
                    .all(&self.txn)
                    .await?
                    .into_iter()
                    .map(|row| ImportedRoom {
                        id: row.id,
                        name: row.name,
                        created_at: row.created_at,
                        seen_at: row.seen_at,
                        seen_through: row.seen_through,
                        project_id: row.project_id,
                    }),
            );
        }
        Ok(found)
    }

    /// Every room's id, in order.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn room_ids(&self) -> Result<Vec<i64>, DbError> {
        Ok(room::Entity::find()
            .order_by_asc(room::Column::Id)
            .all(&self.txn)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect())
    }

    /// Inserts imported messages as they are.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn insert_messages(&self, rows: &[ImportedMessage]) -> Result<(), DbError> {
        for chunk in rows.chunks(BATCH) {
            let models = chunk.iter().map(|row| message::ActiveModel {
                id: Set(row.id),
                room_id: Set(row.room_id),
                author: Set(author_of(row.author)),
                text: Set(row.text.clone()),
                created_at: Set(row.created_at),
                reply_to_id: Set(row.reply_to_id),
            });
            message::Entity::insert_many(models).exec(&self.txn).await?;
        }
        Ok(())
    }

    /// The messages whose id is one of `ids`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn messages_by_id(&self, ids: &[i64]) -> Result<Vec<ImportedMessage>, DbError> {
        let mut found = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(KEYS) {
            found.extend(
                message::Entity::find()
                    .filter(message::Column::Id.is_in(chunk.iter().copied()))
                    .all(&self.txn)
                    .await?
                    .into_iter()
                    .map(|row| ImportedMessage {
                        id: row.id,
                        room_id: row.room_id,
                        author: author_from(row.author),
                        text: row.text,
                        created_at: row.created_at,
                        reply_to_id: row.reply_to_id,
                    }),
            );
        }
        Ok(found)
    }

    /// Inserts imported files as they are.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn insert_files(&self, rows: &[ImportedFile]) -> Result<(), DbError> {
        for chunk in rows.chunks(BATCH) {
            let models = chunk.iter().map(|row| file::ActiveModel {
                id: Set(row.id),
                stored_name: Set(row.stored_name.clone()),
                name: Set(row.name.clone()),
                kind: Set(kind_of(row.kind)),
                size: Set(row.size),
                created_at: Set(row.created_at),
            });
            file::Entity::insert_many(models).exec(&self.txn).await?;
        }
        Ok(())
    }

    /// The files whose id is one of `ids`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn files_by_id(&self, ids: &[i64]) -> Result<Vec<ImportedFile>, DbError> {
        let mut found = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(KEYS) {
            found.extend(
                file::Entity::find()
                    .filter(file::Column::Id.is_in(chunk.iter().copied()))
                    .all(&self.txn)
                    .await?
                    .into_iter()
                    .map(|row| ImportedFile {
                        id: row.id,
                        stored_name: row.stored_name,
                        name: row.name,
                        kind: kind_from(row.kind),
                        size: row.size,
                        created_at: row.created_at,
                    }),
            );
        }
        Ok(found)
    }
}
