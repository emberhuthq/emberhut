//! The machines in one line, for Settings' "Machines" row: `All fine` or `2 to look at`, their
//! names, and a state for its dot.

use emberhut_api::machines::{MachineState, MachinesPage};

/// The row's state, for its dot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    /// Nothing fetched yet.
    Off,
    /// Every machine is fine.
    Fine,
    /// Some machine warns.
    Warn,
    /// Some machine is low or offline.
    Down,
}

/// What the row says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    /// `All fine`, `2 to look at`, or why they couldn't be read (`Looking…` before).
    pub(crate) status: String,
    /// The machines' names.
    pub(crate) detail: String,
    /// The dot's state.
    pub(crate) state: RowState,
}

impl Summary {
    /// The row for the last `page` fetched and the last `problem`.
    #[must_use]
    pub(crate) fn of(page: Option<&MachinesPage>, problem: Option<&str>) -> Self {
        let Some(page) = page else {
            return Self {
                status: problem.unwrap_or("Looking…").to_owned(),
                detail: String::new(),
                state: RowState::Off,
            };
        };
        let look_at: Vec<_> = page
            .machines
            .iter()
            .filter(|machine| {
                matches!(
                    machine.state,
                    MachineState::Warn | MachineState::Low | MachineState::Offline
                )
            })
            .collect();
        let down = look_at
            .iter()
            .any(|machine| matches!(machine.state, MachineState::Low | MachineState::Offline));
        Self {
            status: if look_at.is_empty() {
                "All fine".to_owned()
            } else {
                format!("{} to look at", look_at.len())
            },
            detail: page
                .machines
                .iter()
                .map(|machine| machine.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            state: if look_at.is_empty() {
                RowState::Fine
            } else if down {
                RowState::Down
            } else {
                RowState::Warn
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn page(states: &[&str]) -> MachinesPage {
        let machines: Vec<_> = states
            .iter()
            .enumerate()
            .map(|(i, state)| json!({"id": format!("m{i}"), "name": format!("Machine {i}"), "state": state, "status": ""}))
            .collect();
        serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z", "machines": machines})).unwrap()
    }

    #[test]
    fn all_fine_or_how_many_to_look_at() {
        let fine = Summary::of(Some(&page(&["ok", "stale"])), None);
        assert_eq!(fine.status, "All fine");
        assert_eq!(fine.detail, "Machine 0, Machine 1");
        assert_eq!(fine.state, RowState::Fine);
        let warn = Summary::of(Some(&page(&["ok", "warn"])), None);
        assert_eq!(
            (warn.status.as_str(), warn.state),
            ("1 to look at", RowState::Warn)
        );
        let down = Summary::of(Some(&page(&["warn", "offline"])), None);
        assert_eq!(
            (down.status.as_str(), down.state),
            ("2 to look at", RowState::Down)
        );
    }

    #[test]
    fn before_a_page_it_looks_or_says_why_not() {
        assert_eq!(Summary::of(None, None).status, "Looking…");
        assert_eq!(Summary::of(None, Some("Runner down")).status, "Runner down");
        assert_eq!(Summary::of(None, None).state, RowState::Off);
    }
}
