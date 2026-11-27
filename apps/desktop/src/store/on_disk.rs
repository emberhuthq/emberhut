//! What the app keeps on disk to show a room at once: each room's newest messages, so a room
//! switched to (and the open room, once the server lets this device in) shows its chat before
//! the server answers. They live in the platform's cache directory, one folder per server (two
//! servers' rooms share ids), and the system may clear them: the server has it all. A device
//! that is signed out or refused has its server's folder removed ([`DiskCache::clear`]).
//!
//! Each file is read and written as [`json_file`] does: private to the user, whole through a
//! temporary file and a rename, and numbered per file, so the newest save is what stays.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use emberhut_api::chat::ChatMessage;
use emberhut_api::ids::RoomId;

use super::kept::folder_for;
use crate::config::ServerUrl;
use crate::json_file::{self, Writes};

/// How many of a room's newest messages are kept.
pub(crate) const KEPT_MESSAGES: usize = 200;

/// The files kept for one server.
#[derive(Debug)]
pub(crate) struct DiskCache {
    dir: PathBuf,
    /// The writes to each file.
    writes: Mutex<HashMap<PathBuf, Writes>>,
    /// How many times the folder was cleared: a save asked for before the last clear isn't
    /// written.
    clears: AtomicU64,
    /// Held by each write to read, by a clear to write: no write runs during a clear.
    clearing: RwLock<()>,
}

/// A save of a room's chat, numbered when it was asked for; it is encoded when written, off
/// the UI thread.
#[derive(Debug)]
pub(crate) struct Save {
    path: PathBuf,
    number: u64,
    written: Arc<Mutex<u64>>,
    /// The [`DiskCache::clears`] it was asked for after.
    clears: u64,
    messages: Vec<ChatMessage>,
}

impl DiskCache {
    /// The cache for `server` in the platform's cache directory; `None` without one.
    #[must_use]
    pub(crate) fn locate(server: &ServerUrl) -> Option<Self> {
        Some(Self::in_dir(
            &crate::config::cache_dir()?.join(folder_for(server.as_str())),
        ))
    }

    /// The cache in `dir`.
    #[must_use]
    pub(crate) fn in_dir(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            writes: Mutex::new(HashMap::new()),
            clears: AtomicU64::new(0),
            clearing: RwLock::new(()),
        }
    }

    fn chat_file(&self, room: RoomId) -> PathBuf {
        self.dir.join(format!("chat-{room}.json"))
    }

    /// The messages kept for `room`, oldest first. Reads a file: call it off the UI thread.
    #[must_use]
    pub(crate) fn chat(&self, room: RoomId) -> Option<Vec<ChatMessage>> {
        json_file::read(
            &self.chat_file(room),
            "a kept chat",
            "the room waits for the server",
        )
    }

    /// A save of `room`'s newest [`KEPT_MESSAGES`] messages, to [`write`](Self::write) off the
    /// UI thread.
    #[must_use]
    pub(crate) fn save_chat(&self, room: RoomId, messages: &[ChatMessage]) -> Save {
        let path = self.chat_file(room);
        let (number, written) = self
            .writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(path.clone())
            .or_default()
            .next();
        Save {
            path,
            number,
            written,
            clears: self.clears.load(Ordering::Acquire),
            messages: messages[messages.len().saturating_sub(KEPT_MESSAGES)..].to_vec(),
        }
    }

    /// Writes `save`, unless a newer save of its file was written already or the folder was
    /// cleared since it was asked for.
    pub(crate) fn write(&self, save: &Save) {
        let _writing = self.clearing.read().unwrap_or_else(PoisonError::into_inner);
        if self.clears.load(Ordering::Acquire) != save.clears {
            return;
        }
        json_file::write(
            &save.path,
            &save.messages,
            save.number,
            &save.written,
            "a room's chat",
        );
    }

    /// Removes everything kept for the server: the device was signed out or refused. A save
    /// asked for before this isn't written.
    pub(crate) fn clear(&self) {
        let _clearing = self
            .clearing
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        self.clears.fetch_add(1, Ordering::AcqRel);
        if let Err(error) = std::fs::remove_dir_all(&self.dir)
            && error.kind() != io::ErrorKind::NotFound
        {
            log::warn!("can't remove {}: {error}", self.dir.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::message;

    #[test]
    fn a_rooms_newest_messages_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::in_dir(&dir.path().join("server"));
        let room = RoomId::new(2);
        assert_eq!(cache.chat(room), None, "nothing kept yet");
        let messages: Vec<_> = (1..=250).map(message).collect();
        cache.write(&cache.save_chat(room, &messages));
        let kept = cache.chat(room).unwrap();
        assert_eq!(kept.len(), KEPT_MESSAGES);
        assert_eq!(kept.first().map(|m| m.id.get()), Some(51), "the newest 200");
        assert_eq!(cache.chat(RoomId::MAIN), None, "each room its own");
    }

    #[test]
    fn an_older_save_that_finishes_later_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::in_dir(dir.path());
        let room = RoomId::new(2);
        let older = cache.save_chat(room, &[message(1)]);
        let newer = cache.save_chat(room, &[message(1), message(2)]);
        cache.write(&newer);
        cache.write(&older);
        assert_eq!(cache.chat(room).map(|kept| kept.len()), Some(2));
        assert!(!dir.path().join("chat-2.json.tmp").exists());
    }

    #[test]
    fn a_broken_file_reads_as_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::in_dir(dir.path());
        std::fs::write(dir.path().join("chat-2.json"), "{").unwrap();
        assert_eq!(cache.chat(RoomId::new(2)), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_kept_chats_are_the_users_only() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("server");
        let cache = DiskCache::in_dir(&folder);
        cache.write(&cache.save_chat(RoomId::MAIN, &[message(1)]));
        assert_eq!(crate::private_files::mode(&folder), 0o700);
        assert_eq!(
            crate::private_files::mode(&folder.join("chat-1.json")),
            0o600
        );
    }

    #[test]
    fn clearing_removes_the_servers_folder_and_drops_the_saves_asked_for_before() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("server");
        let cache = DiskCache::in_dir(&folder);
        cache.write(&cache.save_chat(RoomId::MAIN, &[message(1)]));
        let late = cache.save_chat(RoomId::new(2), &[message(2)]);
        cache.clear();
        assert!(!folder.exists());
        cache.write(&late);
        assert!(
            !folder.exists(),
            "a save from before the clear isn't written"
        );
        cache.write(&cache.save_chat(RoomId::MAIN, &[message(3)]));
        assert_eq!(
            cache.chat(RoomId::MAIN).map(|kept| kept.len()),
            Some(1),
            "a save from after it is"
        );
    }

    #[test]
    fn each_server_has_a_folder_of_its_own() {
        let folder = |url: &str| folder_for(ServerUrl::parse(url).unwrap().as_str());
        assert_ne!(
            folder("https://emberhut.example.com"),
            folder("https://emberhut-example.com")
        );
        assert_ne!(folder("http://a:8080"), folder("http://a-8080"));
    }
}
