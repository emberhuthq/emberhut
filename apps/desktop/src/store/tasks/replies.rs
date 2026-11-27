//! The reply box on each task's page: the photos added to it (stored on the server as they
//! are added, four at most), whether it is being sent, and the note under it (where the last
//! message went, or why it wasn't sent).

use std::collections::HashMap;

use emberhut_api::chat::ChatFile;
use emberhut_api::ids::TaskId;

/// How many photos a reply takes.
pub(crate) const MAX_PHOTOS: usize = 4;

/// One task's reply box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reply {
    /// The photos stored for it.
    pub(crate) files: Vec<ChatFile>,
    /// Photos still being stored.
    pub(crate) uploading: usize,
    /// It is on its way.
    pub(crate) sending: bool,
    /// The line under the box.
    pub(crate) note: Option<String>,
    /// How many messages went from it: the view empties its field when this moves.
    pub(crate) sent: u64,
}

impl Reply {
    /// Whether `text` with the photos can be sent now: something to send, nothing still
    /// uploading, nothing on its way.
    #[must_use]
    pub(crate) fn can_send(&self, text: &str) -> bool {
        (!text.trim().is_empty() || !self.files.is_empty()) && self.uploading == 0 && !self.sending
    }

    /// How many more photos it takes.
    #[must_use]
    pub(crate) fn room_for(&self) -> usize {
        MAX_PHOTOS.saturating_sub(self.files.len() + self.uploading)
    }
}

/// The reply boxes, by task.
#[derive(Debug, Default)]
pub(crate) struct Replies {
    by_task: HashMap<TaskId, Reply>,
}

impl Replies {
    /// `task`'s box (an empty one when nothing was done to it).
    #[must_use]
    pub(crate) fn get(&self, task: TaskId) -> Reply {
        self.by_task.get(&task).cloned().unwrap_or_default()
    }

    /// `task`'s box, to change.
    pub(crate) fn of(&mut self, task: TaskId) -> &mut Reply {
        self.by_task.entry(task).or_default()
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::chat::FileKind;

    use super::*;

    #[test]
    fn a_reply_needs_something_to_send_and_nothing_on_its_way() {
        let mut replies = Replies::default();
        let task = TaskId::new(1);
        assert!(!replies.get(task).can_send("  "));
        assert!(replies.get(task).can_send("Shorter, please"));
        replies.of(task).uploading = 1;
        assert!(
            !replies.get(task).can_send("Shorter, please"),
            "a photo is still uploading"
        );
        assert_eq!(replies.get(task).room_for(), 3);
        let reply = replies.of(task);
        reply.uploading = 0;
        reply.files.push(ChatFile {
            name: "shot.png".into(),
            url: "/uploads/shot.png".into(),
            kind: FileKind::Image,
            path: None,
        });
        assert!(replies.get(task).can_send(""), "a photo alone goes");
        replies.of(task).sending = true;
        assert!(!replies.get(task).can_send("more"));
    }
}
