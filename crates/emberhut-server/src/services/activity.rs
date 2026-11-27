//! The activity log: what the assistant and the server did on their own, one line each, for the
//! app's activity list.

use std::sync::Arc;

use emberhut_core::Clock;
use emberhut_db::{Database, DbError};

/// Writes lines to the activity log.
#[derive(Clone, Debug)]
pub struct ActivityLog {
    db: Database,
    clock: Arc<dyn Clock>,
}

impl ActivityLog {
    /// A log in `db`, its lines timed by `clock`.
    #[must_use]
    pub fn new(db: Database, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    /// Adds `text` as a line, now.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn note(&self, text: impl Into<String>) -> Result<(), DbError> {
        self.db.log().add(text, self.clock.now()).await?;
        Ok(())
    }
}
