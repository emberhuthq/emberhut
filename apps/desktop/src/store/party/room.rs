//! The party open in the Party window, as its live stream tells it: the party, what was said,
//! who is speaking word by word, and why the stream or a control failed.

use emberhut_api::ids::PartyId;
use emberhut_api::party::{LineKind, Party, PartyEvent, PartyLine, Speaking};

/// What the stream's thread tells the store.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Heard {
    /// An event.
    Event(Box<PartyEvent>),
    /// The stream broke off; it opens again in a moment.
    Dropped,
    /// The server refused the party (it doesn't have it, or won't show it), in these words:
    /// the stream stops for good.
    Gone(String),
}

/// The party open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Room {
    id: PartyId,
    party: Option<Party>,
    lines: Vec<PartyLine>,
    problem: Option<String>,
}

impl Room {
    /// Party `id`, before its stream has said anything.
    #[must_use]
    pub(crate) fn new(id: PartyId) -> Self {
        Self {
            id,
            party: None,
            lines: Vec::new(),
            problem: None,
        }
    }

    /// Its id.
    #[must_use]
    pub(crate) fn id(&self) -> PartyId {
        self.id
    }

    /// The party, once its stream has sent it.
    #[must_use]
    pub(crate) fn party(&self) -> Option<&Party> {
        self.party.as_ref()
    }

    /// What was said, oldest first.
    #[must_use]
    pub(crate) fn lines(&self) -> &[PartyLine] {
        &self.lines
    }

    /// Who is speaking now and what they have said so far.
    #[must_use]
    pub(crate) fn speaking(&self) -> Option<&Speaking> {
        self.party.as_ref()?.speaking.as_ref()
    }

    /// Why the stream or the last control failed.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Takes what the stream heard in; whether anything changed.
    pub(crate) fn hear(&mut self, heard: Heard) -> bool {
        let event = match heard {
            Heard::Dropped => {
                return replace(&mut self.problem, Some("Reconnecting…".to_owned()));
            }
            Heard::Gone(words) => {
                if let Some(party) = &mut self.party {
                    party.speaking = None;
                }
                return replace(&mut self.problem, Some(words));
            }
            Heard::Event(event) => *event,
        };
        match event {
            PartyEvent::Snapshot { party, messages } => {
                self.party = party;
                self.lines = messages;
                self.problem = None;
                true
            }
            PartyEvent::Party { party: Some(party) } => replace(&mut self.party, Some(party)),
            PartyEvent::Party { party: None } => false,
            PartyEvent::Message { message } => self.said(message),
            PartyEvent::Speaking { speaking } => self
                .party
                .as_mut()
                .is_some_and(|party| replace(&mut party.speaking, speaking)),
        }
    }

    /// A control's answer: the party as it is after, or why it failed.
    pub(crate) fn answered(&mut self, reply: Result<Option<Party>, String>) {
        match reply {
            Ok(Some(party)) => self.party = Some(party),
            Ok(None) => {}
            Err(problem) => self.problem = Some(problem),
        }
    }

    /// A line said: added once; a persona's turn ends its live bubble, so its words don't
    /// show twice.
    fn said(&mut self, line: PartyLine) -> bool {
        if self.lines.iter().any(|held| held.id == line.id) {
            return false;
        }
        if line.kind == LineKind::Turn
            && let Some(party) = &mut self.party
            && party
                .speaking
                .as_ref()
                .is_some_and(|speaking| speaking.who == line.who)
        {
            party.speaking = None;
        }
        self.lines.push(line);
        true
    }
}

/// Puts `new` in `slot`; whether it differed.
fn replace<T: PartialEq>(slot: &mut T, new: T) -> bool {
    let changed = *slot != new;
    *slot = new;
    changed
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::roster::tests::party;
    use super::super::words;
    use super::*;

    fn line(id: i64, who: &str, kind: &str) -> PartyLine {
        serde_json::from_value(json!({
            "id": id, "party": 3, "who": who, "name": "Victor", "kind": kind, "text": "Coffee.",
            "model": null, "tokens": 0, "cost": 0, "cut": false, "at": "2026-11-25T10:00:00.000Z"
        }))
        .unwrap()
    }

    fn ev(event: PartyEvent) -> Heard {
        Heard::Event(Box::new(event))
    }

    fn speaking(who: &str, text: &str) -> Heard {
        ev(PartyEvent::Speaking {
            speaking: Some(Speaking {
                who: who.into(),
                name: "Victor".into(),
                text: text.into(),
            }),
        })
    }

    fn snapshot() -> Heard {
        ev(PartyEvent::Snapshot {
            party: Some(serde_json::from_value(party(3, "running")).unwrap()),
            messages: vec![line(1, "owner", "say")],
        })
    }

    #[test]
    fn a_snapshot_then_each_change() {
        let mut room = Room::new(PartyId::new(3));
        assert!(room.hear(Heard::Dropped));
        assert_eq!(room.problem(), Some("Reconnecting…"));
        assert!(room.hear(snapshot()));
        assert_eq!(room.problem(), None, "connected again");
        assert_eq!(room.lines().len(), 1);
        assert!(room.hear(speaking("p2", "")));
        assert_eq!(room.speaking().unwrap().text, "");
        assert!(room.hear(speaking("p2", "Coffee")));
        assert!(!room.hear(speaking("p2", "Coffee")), "nothing new");
        assert_eq!(
            words::meter(room.party().unwrap()),
            "Turn 5 of 12 · 23.4k of 150.0k tokens · $0.12"
        );
    }

    #[test]
    fn a_turn_landing_ends_its_live_bubble_and_comes_once() {
        let mut room = Room::new(PartyId::new(3));
        room.hear(snapshot());
        room.hear(speaking("p2", "Coffee"));
        room.hear(ev(PartyEvent::Message {
            message: line(2, "p1", "turn"),
        }));
        assert!(room.speaking().is_some(), "another persona's turn");
        room.hear(ev(PartyEvent::Message {
            message: line(3, "p2", "turn"),
        }));
        assert!(room.speaking().is_none());
        assert!(!room.hear(ev(PartyEvent::Message {
            message: line(3, "p2", "turn"),
        })));
        assert_eq!(room.lines().len(), 3);
    }

    #[test]
    fn a_party_event_without_a_party_keeps_the_one_held() {
        let mut room = Room::new(PartyId::new(3));
        room.hear(snapshot());
        assert!(!room.hear(ev(PartyEvent::Party { party: None })));
        let paused = serde_json::from_value(party(3, "paused")).unwrap();
        assert!(room.hear(ev(PartyEvent::Party {
            party: Some(paused)
        })));
        assert_eq!(room.party().unwrap().status.as_str(), "paused");
    }

    #[test]
    fn a_controls_answer_updates_the_party_or_says_why() {
        let mut room = Room::new(PartyId::new(3));
        room.answered(Ok(Some(
            serde_json::from_value(party(3, "stopped")).unwrap(),
        )));
        assert_eq!(room.party().unwrap().status.as_str(), "stopped");
        room.answered(Err("no such party".into()));
        assert_eq!(room.problem(), Some("no such party"));
    }
}
