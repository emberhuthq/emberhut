//! A file the server holds (a chat attachment, a task's result) and what the app does with it.
//! Whatever is done, the whole file comes first, streamed once into the app's folder of kept
//! files (`kept`) however many ask for it at the same time; a file kept already is there at
//! once. Then it is viewed, saved into Downloads or where the user picks, copied, shared, or
//! opened in its default app, the status line saying how far its download is and how each use
//! went ([`status`]).

mod downloads;
mod fetch;
pub(crate) mod kind;
pub(crate) mod status;
pub(crate) mod titles;
mod uses;

pub(crate) use self::downloads::{downloads_dir, save_bytes};
pub(crate) use self::fetch::{KeptFiles, Opening};
pub(crate) use self::kind::Kind;
pub(crate) use self::uses::Use;

/// A file on the server, by its address and its real name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RemoteFile {
    /// Its address on the server (`/uploads/…`, `/api/artifacts/66`).
    pub(crate) path: String,
    /// Its real name, as shown and as saved.
    pub(crate) name: String,
}

impl RemoteFile {
    /// The file at `path`, called `name`.
    #[must_use]
    pub(crate) fn new(path: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
        }
    }

    /// What kind of file it is, from its name.
    #[must_use]
    pub(crate) fn kind(&self) -> Kind {
        Kind::of(&self.name)
    }

    /// Whether it could run as a program when the system opens it, by its name or by its
    /// address ([`kind::runs_as_program`]): it is then only ever shown in its folder.
    #[must_use]
    pub(crate) fn runs_as_program(&self) -> bool {
        kind::runs_as_program(&self.name) || kind::runs_as_program(&self.path)
    }
}
