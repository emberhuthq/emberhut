//! A new party being put together: its cast (ready-made personas, the user's own from earlier
//! parties, or new ones), each one's model, and the caps, with the most it could cost.

use emberhut_api::party::{CastMember, MAX_CAST, MIN_CAST, NewParty, PartyModel, Persona};

/// The model a persona talks with unless another is picked: the reference server's default.
pub(crate) const DEFAULT_MODEL: &str = "claude-opus-5-5";

/// The turn caps offered: 2 to 60, by 2.
pub(crate) const TURNS: (u32, u32, u32) = (2, 60, 2);

/// The token caps offered.
pub(crate) const TOKEN_CAPS: [u64; 4] = [50_000, 150_000, 500_000, 1_000_000];

/// The ready-made personas a new party starts with: the optimist, the skeptic, the engineer.
const STARTING_CAST: [&str; 3] = ["optimist", "skeptic", "engineer"];

/// A persona's place in the draft's cast, so an edit finds it again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CastKey(u64);

/// A persona in the cast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Member {
    /// Its place.
    pub(crate) key: CastKey,
    /// Its name.
    pub(crate) name: String,
    /// Who they are and how they talk.
    pub(crate) role: String,
    /// Its model's id.
    pub(crate) model: String,
}

/// A persona being edited: one of the cast (`key`), or a new one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Editing {
    /// The cast member it changes; none for a new persona.
    pub(crate) key: Option<CastKey>,
    /// The model picked.
    pub(crate) model: String,
}

/// The new party's form, but for its topic (a text field of the view).
#[derive(Clone, Debug)]
pub(crate) struct Draft {
    cast: Vec<Member>,
    next: u64,
    /// The turn cap.
    pub(crate) max_turns: u32,
    /// The token cap.
    pub(crate) max_tokens: u64,
    /// The persona being edited, if one is.
    pub(crate) editing: Option<Editing>,
    /// Starting now.
    pub(crate) busy: bool,
    /// Why it couldn't start, in the server's words.
    pub(crate) problem: Option<String>,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            cast: Vec::new(),
            next: 0,
            max_turns: 12,
            max_tokens: 150_000,
            editing: None,
            busy: false,
            problem: None,
        }
    }
}

impl Draft {
    /// A draft with the starting cast, from the ready-made `presets`.
    #[must_use]
    pub(crate) fn new(presets: &[Persona]) -> Self {
        let mut draft = Self::default();
        draft.start_from(presets);
        draft
    }

    /// Fills an empty cast with the starting personas from `presets` (the list may have come
    /// after the form opened).
    pub(crate) fn start_from(&mut self, presets: &[Persona]) {
        if !self.cast.is_empty() {
            return;
        }
        for id in STARTING_CAST {
            if let Some(persona) = presets.iter().find(|persona| persona.id.as_str() == id) {
                self.add(persona);
            }
        }
    }

    /// The cast.
    #[must_use]
    pub(crate) fn cast(&self) -> &[Member] {
        &self.cast
    }

    /// The cast member `key`.
    #[must_use]
    pub(crate) fn member(&self, key: CastKey) -> Option<&Member> {
        self.cast.iter().find(|member| member.key == key)
    }

    /// Whether another persona fits.
    #[must_use]
    pub(crate) fn has_room(&self) -> bool {
        self.cast.len() < MAX_CAST
    }

    /// Of `personas`, the ones not in the cast already (by name).
    #[must_use]
    pub(crate) fn offered<'a>(&self, personas: &'a [Persona]) -> Vec<&'a Persona> {
        personas
            .iter()
            .filter(|persona| !self.cast.iter().any(|member| member.name == persona.name))
            .collect()
    }

    /// Adds `persona` to the cast, if it has room; a persona without a model talks with the
    /// default.
    pub(crate) fn add(&mut self, persona: &Persona) {
        self.push(&persona.name, &persona.role, &persona.model);
    }

    /// Takes the cast member `key` out.
    pub(crate) fn remove(&mut self, key: CastKey) {
        self.cast.retain(|member| member.key != key);
    }

    /// Saves the persona edited as `name`, `role` and the model picked: over its cast member,
    /// or added as a new one if the cast has room. Nothing without a name.
    pub(crate) fn save(&mut self, name: &str, role: &str) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        match editing
            .key
            .and_then(|key| self.cast.iter_mut().find(|member| member.key == key))
        {
            Some(member) => {
                name.clone_into(&mut member.name);
                role.clone_into(&mut member.role);
                member.model = editing.model;
            }
            None => self.push(name, role, &editing.model),
        }
    }

    /// Opens the editor on cast member `key`, or on a new persona.
    pub(crate) fn edit(&mut self, key: Option<CastKey>) {
        let model = key
            .and_then(|key| self.member(key))
            .map_or_else(|| DEFAULT_MODEL.to_owned(), |member| member.model.clone());
        self.editing = Some(Editing { key, model });
    }

    /// One step more or fewer turns, within [`TURNS`].
    pub(crate) fn step_turns(&mut self, up: bool) {
        let (least, most, step) = TURNS;
        self.max_turns = if up {
            (self.max_turns + step).min(most)
        } else {
            self.max_turns.saturating_sub(step).max(least)
        };
    }

    /// Whether Start can go: a topic, at least [`MIN_CAST`] personas, and not starting already.
    #[must_use]
    pub(crate) fn can_start(&self, topic: &str) -> bool {
        !self.busy && !topic.trim().is_empty() && self.cast.len() >= MIN_CAST
    }

    /// The most the party could cost: the token cap, all at the dearest model in the cast,
    /// a fifth of the tokens priced as output. A member whose model isn't in `models` is
    /// priced as the cheapest one there, as the reference client prices a model it doesn't
    /// know; `None` while no model's price is known (the list isn't read yet).
    #[must_use]
    pub(crate) fn estimate(&self, models: &[PartyModel]) -> Option<f64> {
        let price = |model: &PartyModel| 0.8 * model.price_in + 0.2 * model.price_out;
        let cheapest = models.iter().map(price).reduce(f64::min)?;
        let dearest = self
            .cast
            .iter()
            .map(|member| {
                models
                    .iter()
                    .find(|model| model.id == member.model)
                    .map_or(cheapest, price)
            })
            .fold(0.0, f64::max);
        #[expect(
            clippy::cast_precision_loss,
            reason = "a token cap of at most a million, shown as dollars"
        )]
        let tokens = self.max_tokens as f64;
        Some(tokens * dearest / 1_000_000.0)
    }

    /// The request that starts it, about `topic`.
    #[must_use]
    pub(crate) fn request(&self, topic: &str) -> NewParty {
        NewParty {
            topic: topic.trim().to_owned(),
            max_turns: self.max_turns,
            max_tokens: self.max_tokens,
            personas: self
                .cast
                .iter()
                .map(|member| CastMember {
                    name: member.name.clone(),
                    role: member.role.clone(),
                    model: member.model.clone(),
                })
                .collect(),
            title: None,
        }
    }

    fn push(&mut self, name: &str, role: &str, model: &str) {
        if !self.has_room() {
            return;
        }
        self.next += 1;
        self.cast.push(Member {
            key: CastKey(self.next),
            name: name.to_owned(),
            role: role.to_owned(),
            model: if model.is_empty() {
                DEFAULT_MODEL.to_owned()
            } else {
                model.to_owned()
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::roster::tests::page;
    use super::*;

    #[test]
    fn a_new_party_starts_with_the_optimist_the_skeptic_and_the_engineer() {
        let page = page();
        let draft = Draft::new(&page.presets);
        let names: Vec<_> = draft.cast().iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Nora", "Victor", "Sam"]);
        assert_eq!(
            draft.cast()[2].model,
            DEFAULT_MODEL,
            "a preset without a model"
        );
        assert_eq!(draft.max_turns, 12);
        assert_eq!(draft.max_tokens, 150_000);
        let offered: Vec<_> = draft
            .offered(&page.presets)
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(offered, ["Jules"], "those in the cast aren't offered again");
    }

    #[test]
    fn the_cast_holds_five_at_most() {
        let page = page();
        let mut draft = Draft::new(&page.presets);
        draft.add(&page.presets[3]);
        draft.add(&page.recent[0]);
        assert!(!draft.has_room());
        draft.add(&page.presets[0]);
        assert_eq!(draft.cast().len(), 5);
        draft.edit(None);
        draft.save("Extra", "");
        assert_eq!(draft.cast().len(), 5, "a new one doesn't fit either");
    }

    #[test]
    fn an_edit_changes_its_member_and_a_new_one_joins() {
        let page = page();
        let mut draft = Draft::new(&page.presets);
        let sam = draft.cast()[2].key;
        draft.edit(Some(sam));
        assert_eq!(draft.editing.as_ref().unwrap().model, DEFAULT_MODEL);
        draft.editing.as_mut().unwrap().model = "claude-haiku-5-5".into();
        draft.save("  Sam  ", "Builds things.");
        let edited = draft.member(sam).unwrap();
        assert_eq!(
            (edited.name.as_str(), edited.model.as_str()),
            ("Sam", "claude-haiku-5-5")
        );
        draft.edit(None);
        draft.save(" ", "No name");
        assert_eq!(draft.cast().len(), 3, "Done needs a name");
        assert!(draft.editing.is_none());
        draft.edit(None);
        draft.save("Ada", "");
        assert_eq!(draft.cast().len(), 4);
        draft.remove(sam);
        assert!(draft.member(sam).is_none());
    }

    #[test]
    fn start_needs_a_topic_and_two_personas() {
        let page = page();
        let mut draft = Draft::new(&page.presets);
        assert!(!draft.can_start("  "));
        assert!(draft.can_start("Tea or coffee"));
        let keys: Vec<_> = draft.cast().iter().map(|m| m.key).collect();
        draft.remove(keys[0]);
        draft.remove(keys[1]);
        assert!(!draft.can_start("Tea or coffee"));
        draft.add(&page.presets[3]);
        draft.busy = true;
        assert!(!draft.can_start("Tea or coffee"), "not while it starts");
    }

    #[test]
    fn turns_step_by_two_within_their_bounds() {
        let mut draft = Draft::default();
        draft.step_turns(true);
        assert_eq!(draft.max_turns, 14);
        draft.max_turns = 60;
        draft.step_turns(true);
        assert_eq!(draft.max_turns, 60);
        draft.max_turns = 2;
        draft.step_turns(false);
        assert_eq!(draft.max_turns, 2);
    }

    #[test]
    fn the_estimate_prices_the_cap_at_the_dearest_model() {
        let page = page();
        let mut draft = Draft::new(&page.presets);
        // Opus: 150k tokens at 0.8 × $4 + 0.2 × $20 per million.
        assert!((draft.estimate(&page.models).unwrap() - 1.08).abs() < 1e-9);
        let keys: Vec<_> = draft.cast().iter().map(|m| m.key).collect();
        for key in keys {
            draft.remove(key);
        }
        draft.add(&page.presets[3]);
        draft.max_tokens = 1_000_000;
        assert!((draft.estimate(&page.models).unwrap() - 0.18).abs() < 1e-9);
        assert_eq!(draft.estimate(&[]), None, "no prices known yet");
    }

    #[test]
    fn a_model_off_the_list_is_priced_as_the_cheapest_one() {
        let page = page();
        let mut draft = Draft::new(&page.presets);
        let keys: Vec<_> = draft.cast().iter().map(|m| m.key).collect();
        for key in keys {
            draft.remove(key);
        }
        draft.add(&page.presets[3]);
        let cheapest = draft.estimate(&page.models).unwrap();
        let key = draft.cast()[0].key;
        draft.edit(Some(key));
        if let Some(editing) = &mut draft.editing {
            "a-model-nobody-lists".clone_into(&mut editing.model);
        }
        draft.save("Someone", "A role");
        let unknown = draft.estimate(&page.models).unwrap();
        assert!(unknown > 0., "not free");
        assert!(unknown <= cheapest + 1e-9, "the cheapest price, at most");
    }

    #[test]
    fn the_request_carries_the_trimmed_topic_and_the_cast() {
        let page = page();
        let draft = Draft::new(&page.presets);
        let request = draft.request("  Tea or coffee \n");
        assert_eq!(request.topic, "Tea or coffee");
        assert_eq!(request.personas.len(), 3);
        assert_eq!(request.personas[1].name, "Victor");
        assert_eq!(request.max_tokens, 150_000);
    }
}
