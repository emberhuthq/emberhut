//! Who watches a session live (`GET /sessions/{id}/live`, server-sent events): each new step,
//! the words as they're typed and the session's head go to every open stream at once. Nothing is
//! polled; a watcher that drops comes back with its last step and misses nothing.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;
use tokio::sync::mpsc;

/// The open live streams, by session. Each gets ready `data: …` frames.
#[derive(Debug, Default)]
pub struct Watchers {
    by: Mutex<HashMap<String, Vec<mpsc::UnboundedSender<String>>>>,
}

impl Watchers {
    /// Whether anyone watches the session.
    #[must_use]
    pub fn has(&self, id: &str) -> bool {
        self.by
            .lock()
            .is_ok_and(|m| m.get(id).is_some_and(|v| v.iter().any(|t| !t.is_closed())))
    }

    /// Sends one event to everyone watching the session (a stream that closed is dropped).
    pub fn send(&self, id: &str, payload: &Value) {
        let Ok(mut m) = self.by.lock() else { return };
        let Some(list) = m.get_mut(id) else { return };
        let frame = format!("data: {payload}\n\n");
        list.retain(|t| t.send(frame.clone()).is_ok());
        if list.is_empty() {
            m.remove(id);
        }
    }

    /// A new stream for the session: its frames arrive on the receiver until it is dropped.
    #[must_use]
    pub fn add(&self, id: &str) -> mpsc::UnboundedReceiver<String> {
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut m) = self.by.lock() {
            m.entry(id.to_owned()).or_default().push(tx);
        }
        rx
    }

    /// Ends every stream.
    pub fn close_all(&self) {
        if let Ok(mut m) = self.by.lock() {
            m.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_watcher_gets_each_event_and_a_closed_one_is_dropped() {
        let w = Watchers::default();
        let mut a = w.add("r-1");
        let b = w.add("r-1");
        drop(b);
        w.send("r-1", &serde_json::json!({"type": "tokens", "tokens": 3}));
        let frame = a.try_recv().unwrap();
        let body = frame
            .strip_prefix("data: ")
            .and_then(|f| f.strip_suffix("\n\n"))
            .unwrap();
        // Key order depends on serde_json's features in the build; the value is what counts.
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap(),
            serde_json::json!({"type": "tokens", "tokens": 3})
        );
        assert!(w.has("r-1") && !w.has("r-2"));
    }
}
