//! The Party window's list: the parties, the ready-made personas, the user's own and the
//! models, as last read, and why the last read failed.

use emberhut_api::ids::PartyId;
use emberhut_api::party::{PartiesPage, Party, PartyModel, Persona};

use super::words;
use crate::backend::Error;

/// The list as last read.
#[derive(Debug, Default)]
pub(crate) struct Roster {
    page: Option<PartiesPage>,
    problem: Option<String>,
}

impl Roster {
    /// Whether a read has come in.
    #[must_use]
    pub(crate) fn is_loaded(&self) -> bool {
        self.page.is_some()
    }

    /// Why the last read failed, worded for the user.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The parties, the one changed last first.
    #[must_use]
    pub(crate) fn parties(&self) -> &[Party] {
        self.page
            .as_ref()
            .map_or(&[], |page| page.parties.as_slice())
    }

    /// The ready-made personas.
    #[must_use]
    pub(crate) fn presets(&self) -> &[Persona] {
        self.page
            .as_ref()
            .map_or(&[], |page| page.presets.as_slice())
    }

    /// The user's own personas from earlier parties.
    #[must_use]
    pub(crate) fn recent(&self) -> &[Persona] {
        self.page
            .as_ref()
            .map_or(&[], |page| page.recent.as_slice())
    }

    /// The models a persona can talk with.
    #[must_use]
    pub(crate) fn models(&self) -> &[PartyModel] {
        self.page
            .as_ref()
            .map_or(&[], |page| page.models.as_slice())
    }

    /// A model's name, from the server's list or else its id.
    #[must_use]
    pub(crate) fn model_name(&self, id: &str) -> String {
        self.models()
            .iter()
            .find(|model| model.id == id)
            .map_or_else(|| words::model_fallback(id), |model| model.name.clone())
    }

    /// Takes a read in. A failure keeps what was read before.
    pub(super) fn take(&mut self, page: Result<PartiesPage, Error>) {
        match page {
            Ok(page) => (self.page, self.problem) = (Some(page), None),
            Err(error) => self.problem = Some(problem(&error)),
        }
    }

    /// Drops party `id` from the list at once (it is being deleted).
    pub(super) fn remove(&mut self, id: PartyId) {
        if let Some(page) = &mut self.page {
            page.parties.retain(|party| party.id != id);
        }
    }
}

/// What a failed read of the parties says.
fn problem(error: &Error) -> String {
    match error {
        Error::Refused {
            status: Some(404), ..
        } => "The server is on older code: Party comes with its next update".to_owned(),
        Error::Unreachable(_) => format!("Can't reach {}", crate::brand::APP_NAME),
        _ => "Couldn't read your parties".to_owned(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::*;

    /// A page with two parties, three presets, one of the user's own and the models.
    pub(crate) fn page() -> PartiesPage {
        serde_json::from_value(json!({
            "parties": [party(2, "running"), party(1, "done")],
            "presets": [
                {"id": "optimist", "name": "Nora", "role": "An optimist. Builds on ideas.", "model": "claude-opus-5-5", "color": ""},
                {"id": "skeptic", "name": "Victor", "role": "A skeptic.", "model": "claude-opus-5-5", "color": ""},
                {"id": "engineer", "name": "Sam", "role": "A practical engineer.", "model": "", "color": ""},
                {"id": "comedian", "name": "Jules", "role": "A comedian.", "model": "claude-haiku-5-5", "color": ""}
            ],
            "recent": [{"id": "recent1", "name": "Ada", "role": "A reviewer.", "model": "claude-sonnet-5-5", "color": ""}],
            "models": [
                {"id": "claude-opus-5-5", "name": "Opus 5.5", "in": 4, "out": 20, "note": "Best talker"},
                {"id": "claude-sonnet-5-5", "name": "Sonnet 5.5", "in": 2, "out": 10, "note": "Half the price"},
                {"id": "claude-haiku-5-5", "name": "Haiku 5.5", "in": 0.1, "out": 0.5, "note": "Fast and cheap"}
            ],
            "defaults": {"maxTurns": 12, "maxTokens": 150_000, "model": "claude-opus-5-5"}
        }))
        .expect("a page")
    }

    /// Party `id`'s JSON, in `status`, with Nora and Victor.
    pub(crate) fn party(id: i64, status: &str) -> serde_json::Value {
        json!({
            "id": id, "title": format!("Party {id}"), "topic": "Tea or coffee", "status": status,
            "why": null,
            "personas": [
                {"id": "p1", "name": "Nora", "role": "An optimist.", "model": "claude-opus-5-5", "color": "blue"},
                {"id": "p2", "name": "Victor", "role": "A skeptic.", "model": "claude-sonnet-5-5", "color": "orange"}
            ],
            "turns": 5, "maxTurns": 12, "tokens": 23_400, "maxTokens": 150_000, "cost": 0.12,
            "created": "2026-11-25T10:00:00.000Z", "updated": "2026-11-25T10:00:00.000Z",
            "speaking": null, "last": null
        })
    }

    #[test]
    fn a_read_fills_it_and_a_failure_keeps_it() {
        let mut roster = Roster::default();
        assert!(!roster.is_loaded());
        roster.take(Ok(page()));
        assert_eq!(roster.parties().len(), 2);
        assert_eq!(roster.model_name("claude-haiku-5-5"), "Haiku 5.5");
        assert_eq!(roster.model_name("claude-mystery-1"), "Mystery-1");
        roster.take(Err(Error::Unreachable("down".into())));
        assert_eq!(roster.parties().len(), 2, "what was read stays");
        assert_eq!(roster.problem(), Some("Can't reach Emberhut"));
        roster.take(Ok(page()));
        assert_eq!(roster.problem(), None);
    }

    #[test]
    fn each_failure_has_its_words() {
        let older = Error::Refused {
            status: Some(404),
            message: None,
        };
        assert!(problem(&older).contains("older code"));
        assert_eq!(
            problem(&Error::Unreadable("x".into())),
            "Couldn't read your parties"
        );
    }

    #[test]
    fn a_deleted_party_leaves_the_list_at_once() {
        let mut roster = Roster::default();
        roster.take(Ok(page()));
        roster.remove(PartyId::new(2));
        let left: Vec<_> = roster.parties().iter().map(|party| party.id).collect();
        assert_eq!(left, [PartyId::new(1)]);
    }
}
