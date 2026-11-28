//! What every database test needs: a fresh database in its own directory.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use emberhut_core::OsRandom;
use emberhut_core::session::IssuedTokens;
use emberhut_db::{Database, DatabaseUrl};
use jiff::{SignedDuration, Timestamp};
use tempfile::TempDir;

/// A new, migrated database in a directory that is deleted with the returned guard.
pub(crate) async fn database() -> (TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let db = reopen(&dir).await;
    (dir, db)
}

/// A pool on the database in `dir`, as another process would open it.
pub(crate) async fn reopen(dir: &TempDir) -> Database {
    let url: DatabaseUrl = DatabaseUrl::from_path(&dir.path().join("emberhut.db")).unwrap();
    Database::connect(&url).await.unwrap()
}

/// An instant `minutes` after a fixed start, so tests read as a timeline.
pub(crate) fn at(minutes: i64) -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap() + SignedDuration::from_mins(minutes)
}

/// Fresh tokens issued [`at`] `minutes`.
pub(crate) fn tokens(minutes: i64) -> IssuedTokens {
    IssuedTokens::issue(&OsRandom, at(minutes)).unwrap()
}
