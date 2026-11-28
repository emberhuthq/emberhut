//! `PATCH /api/tasks/{id}`: the person renames a task. The answer is the task as the lists show
//! it ([`Issue`](crate::tasks::Issue)), and its history says `Renamed: <old> → <new>`. A title
//! that is blank or longer than [`TITLE_MAX`] characters (once trimmed) is 400, a task the
//! server doesn't have 404, each with an [`ErrorBody`](crate::error::ErrorBody); it needs a
//! signed-in device.

use serde::{Deserialize, Serialize};

/// The longest a title given in a rename may be, in characters.
pub const TITLE_MAX: usize = 200;

/// `PATCH /api/tasks/{id}`'s body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RenameTask {
    /// The new title: 1 to [`TITLE_MAX`] characters, trimmed by the server.
    pub title: String,
}
