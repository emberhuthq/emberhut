//! The runners paired at run time, as the server reaches them: their rows
//! (`emberhut_db::PairedRunnerStore`), kept in memory too.
//!
//! The runner registry asks for a runner's name, words, address and token on every request and
//! every report, some of them where it can't wait for the database (the runners' names and
//! words are read without `await`), so the rows are kept in memory: read once when the server
//! starts ([`PairedRunners::load`]), then changed with every change this server makes to the
//! table ([`PairedRunners::keep`], [`PairedRunners::drop_runner`]), after the database's. The
//! server is the only writer of the table, so the copy is never older than the last change.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use emberhut_db::{DbError, PairedRunner, PairedRunnerStore};
use futures_util::future::BoxFuture;

use crate::services::runner_api::RunnerBook;

/// The paired runners, from the database, kept in memory. Cheap to clone: clones share the
/// copy.
#[derive(Clone, Debug)]
pub struct PairedRunners {
    store: PairedRunnerStore,
    rows: Arc<RwLock<Vec<PairedRunner>>>,
}

impl PairedRunners {
    /// The paired runners of `store`, none read yet ([`Self::load`] reads them).
    #[must_use]
    pub fn new(store: PairedRunnerStore) -> Self {
        Self {
            store,
            rows: Arc::default(),
        }
    }

    /// The store the rows are read from.
    #[must_use]
    pub fn store(&self) -> &PairedRunnerStore {
        &self.store
    }

    /// Reads every paired runner, as the server starts.
    ///
    /// # Errors
    ///
    /// [`DbError`]; the copy is left as it was.
    pub async fn load(&self) -> Result<(), DbError> {
        let rows = self.store.list().await?;
        *self.rows.write().unwrap_or_else(PoisonError::into_inner) = rows;
        Ok(())
    }

    /// Keeps `runner` as the database now has it: a new runner, or one of the same name in
    /// place of the old.
    pub fn keep(&self, runner: PairedRunner) {
        let mut rows = self.rows.write().unwrap_or_else(PoisonError::into_inner);
        rows.retain(|row| row.pairing.name != runner.pairing.name);
        let at = rows.partition_point(|row| row.pairing.name < runner.pairing.name);
        rows.insert(at, runner);
    }

    /// Drops the runner `name`, as the database has.
    pub fn drop_runner(&self, name: &str) {
        self.rows
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|row| row.pairing.name != name);
    }

    /// Every paired runner, by name.
    #[must_use]
    pub fn list(&self) -> Vec<PairedRunner> {
        self.rows
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// What `f` makes of the runner `name`, if one is paired.
    fn with<T>(&self, name: &str, f: impl FnOnce(&PairedRunner) -> T) -> Option<T> {
        self.rows
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|row| row.pairing.name == name)
            .map(f)
    }
}

impl RunnerBook for PairedRunners {
    fn names(&self) -> Vec<String> {
        self.rows
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|row| row.pairing.name.clone())
            .collect()
    }

    fn words(&self, name: &str) -> Option<String> {
        self.with(name, |row| row.pairing.words.clone()).flatten()
    }

    fn url(&self, name: &str) -> Option<String> {
        self.with(name, |row| row.pairing.url.clone())
    }

    fn tokens(&self) -> BoxFuture<'_, BTreeMap<String, String>> {
        let tokens = self
            .rows
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|row| (row.pairing.name.clone(), row.pairing.token.clone()))
            .collect();
        Box::pin(async move { tokens })
    }
}
