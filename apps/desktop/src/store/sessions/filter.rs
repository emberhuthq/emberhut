//! The Sessions screen's filters: a room (one pick), who runs it (several picks: the
//! assistant's worker, a runner session, an ultra lead, its helper agents, the user's own
//! sessions, an external thread) and where (several machines and accounts). Picks in one menu add up
//! (Mac or the workstation); the menus narrow each other (Mac and one account). Each pick
//! counts what it would show with the other menus as they are: a menu ignores its own picks.

use std::collections::BTreeSet;

use emberhut_api::ids::RoomId;
use emberhut_api::runner::agents::{AgentNow, AgentWho};
use emberhut_api::threads::ExternalThread;
use emberhut_api::work::RunnerSession;
use serde::{Deserialize, Serialize};

use super::words::machine_name;

/// Who runs a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Who {
    /// The assistant's worker on a task.
    Worker,
    /// A runner session the assistant runs.
    Session,
    /// An ultra run's lead.
    Lead,
    /// A lead's helper agents.
    Helper,
    /// The user's own sessions.
    Direct,
    /// An external thread.
    CodingApp,
}

impl Who {
    /// Every kind, in the menu's order.
    pub(crate) const ALL: [Self; 6] = [
        Self::Worker,
        Self::Session,
        Self::Lead,
        Self::Helper,
        Self::Direct,
        Self::CodingApp,
    ];

    /// Its name in the menu.
    #[must_use]
    pub(crate) fn name(self) -> String {
        match self {
            Self::Worker => format!("{} worker", crate::brand::APP_NAME),
            Self::Session => "Runner session".to_owned(),
            Self::Lead => "Ultra lead".to_owned(),
            Self::Helper => "Helper agents".to_owned(),
            Self::Direct => "Your sessions".to_owned(),
            Self::CodingApp => "External thread".to_owned(),
        }
    }

    fn of(who: &AgentWho) -> Self {
        match who {
            AgentWho::Worker => Self::Worker,
            AgentWho::Lead => Self::Lead,
            AgentWho::Direct => Self::Direct,
            AgentWho::Helper => Self::Helper,
            _ => Self::Session,
        }
    }
}

/// What the filters read off a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Facts {
    /// Who runs it.
    pub(crate) worker: Who,
    /// The runner it is on.
    pub(crate) machine: Option<String>,
    /// Its account, by name (the same account on two machines is one).
    pub(crate) account: Option<String>,
    /// Its machine as the server names it (`Mac`), when it does.
    pub(crate) place: Option<String>,
}

impl Facts {
    /// A working agent's.
    #[must_use]
    pub(crate) fn of_agent(agent: &AgentNow) -> Self {
        Self {
            worker: Who::of(&agent.who()),
            machine: agent.machine.as_ref().map(ToString::to_string),
            account: agent.account_label().map(str::to_owned),
            place: agent.place.clone(),
        }
    }

    /// A session's.
    #[must_use]
    pub(crate) fn of_session(session: &RunnerSession) -> Self {
        Self {
            worker: if session.direct {
                Who::Direct
            } else {
                Who::Session
            },
            machine: Some(session.machine.to_string()),
            account: session.account_label().map(str::to_owned),
            place: Some(session.place.clone()),
        }
    }

    /// An external thread's: the runner of its machine, no account.
    #[must_use]
    pub(crate) fn of_thread(thread: &ExternalThread) -> Self {
        Self {
            worker: Who::CodingApp,
            machine: Some(thread.machine.to_string()),
            account: None,
            place: Some(thread.place().to_owned()),
        }
    }
}

/// A part of the filter a menu sets, left out when the menu counts its own picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Worker,
    Machine,
    Account,
}

/// The group a pick is listed under in the Environment menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    /// The machines.
    Machine,
    /// The accounts.
    Account,
}

impl Group {
    /// Its header in the menu.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Machine => "Hut",
            Self::Account => "Account",
        }
    }
}

/// What a pick sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    /// Who runs it.
    Worker(Who),
    /// A machine.
    Machine(String),
    /// An account, by name.
    Account(String),
}

/// One pick of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pick {
    /// What it sets.
    pub(crate) choice: Choice,
    /// Its words.
    pub(crate) label: String,
    /// How many rows it would show with the other menus as they are.
    pub(crate) count: usize,
    /// Its group in the Environment menu.
    pub(crate) group: Option<Group>,
    /// It is picked.
    pub(crate) on: bool,
}

/// A menu's picks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Picks {
    /// Who runs it: each kind that has a row (or is picked).
    pub(crate) workers: Vec<Pick>,
    /// Where: each machine, then each account, that has a row (or is picked).
    pub(crate) environment: Vec<Pick>,
}

/// The filter. An empty set narrows nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionFilter {
    /// One room's rows only.
    #[serde(default)]
    pub(crate) room: Option<RoomId>,
    /// Rows run by one of these.
    #[serde(default)]
    pub(crate) workers: BTreeSet<Who>,
    /// Rows on one of these machines.
    #[serde(default)]
    pub(crate) machines: BTreeSet<String>,
    /// Rows on one of these accounts.
    #[serde(default)]
    pub(crate) accounts: BTreeSet<String>,
}

impl SessionFilter {
    /// Whether it narrows nothing.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// How many menus narrow: room, worker and environment (machines and accounts are one).
    #[must_use]
    pub(crate) fn count(&self) -> usize {
        usize::from(self.room.is_some())
            + usize::from(!self.workers.is_empty())
            + usize::from(!self.machines.is_empty() || !self.accounts.is_empty())
    }

    /// Whether the worker and environment menus keep a row (the room is the caller's: rows
    /// know their room differently).
    #[must_use]
    pub(crate) fn keeps(&self, facts: &Facts) -> bool {
        self.keeps_but(facts, None)
    }

    fn keeps_but(&self, facts: &Facts, skip: Option<Part>) -> bool {
        let worker = skip == Some(Part::Worker)
            || self.workers.is_empty()
            || self.workers.contains(&facts.worker);
        let machine = skip == Some(Part::Machine)
            || self.machines.is_empty()
            || facts
                .machine
                .as_ref()
                .is_some_and(|m| self.machines.contains(m));
        let account = skip == Some(Part::Account)
            || self.accounts.is_empty()
            || facts
                .account
                .as_ref()
                .is_some_and(|a| self.accounts.contains(a));
        worker && machine && account
    }

    /// The menus' picks for these rows (already narrowed by the room).
    #[must_use]
    pub(crate) fn picks(&self, rows: &[Facts]) -> Picks {
        let count = |part: Part, hit: &dyn Fn(&Facts) -> bool| {
            rows.iter()
                .filter(|row| self.keeps_but(row, Some(part)) && hit(row))
                .count()
        };
        let workers = Who::ALL
            .into_iter()
            .filter_map(|worker| {
                let n = count(Part::Worker, &|row| row.worker == worker);
                let on = self.workers.contains(&worker);
                (n > 0 || on).then(|| Pick {
                    choice: Choice::Worker(worker),
                    label: worker.name(),
                    count: n,
                    group: None,
                    on,
                })
            })
            .collect();
        let mut machines: BTreeSet<String> =
            rows.iter().filter_map(|row| row.machine.clone()).collect();
        machines.extend(self.machines.iter().cloned());
        // The runner an account with no machine is on first, then the others by name.
        let mut ordered: Vec<String> = machines.into_iter().collect();
        ordered.sort_by_key(|machine| {
            (
                machine != emberhut_api::runner::accounts::LINUX_RUNNER,
                machine.clone(),
            )
        });
        let mut environment: Vec<Pick> = ordered
            .into_iter()
            .map(|machine| Pick {
                label: rows
                    .iter()
                    .filter(|row| row.machine.as_ref() == Some(&machine))
                    .filter_map(|row| row.place.as_deref())
                    .find(|place| !place.trim().is_empty())
                    .unwrap_or_else(|| machine_name(&machine))
                    .to_owned(),
                count: count(Part::Machine, &|row| row.machine.as_ref() == Some(&machine)),
                group: Some(Group::Machine),
                on: self.machines.contains(&machine),
                choice: Choice::Machine(machine),
            })
            .collect();
        let mut accounts: BTreeSet<String> =
            rows.iter().filter_map(|row| row.account.clone()).collect();
        accounts.extend(self.accounts.iter().cloned());
        environment.extend(accounts.into_iter().map(|account| Pick {
            label: account.clone(),
            count: count(Part::Account, &|row| row.account.as_ref() == Some(&account)),
            group: Some(Group::Account),
            on: self.accounts.contains(&account),
            choice: Choice::Account(account),
        }));
        Picks {
            workers,
            environment,
        }
    }

    /// Picks `choice`, or takes it off.
    pub(crate) fn toggle(&mut self, choice: &Choice) {
        fn flip<T: Ord + Clone>(set: &mut BTreeSet<T>, item: &T) {
            if !set.remove(item) {
                set.insert(item.clone());
            }
        }
        match choice {
            Choice::Worker(worker) => flip(&mut self.workers, worker),
            Choice::Machine(machine) => flip(&mut self.machines, machine),
            Choice::Account(account) => flip(&mut self.accounts, account),
        }
    }

    /// Clears the worker menu.
    pub(crate) fn clear_workers(&mut self) {
        self.workers.clear();
    }

    /// Clears the environment menu.
    pub(crate) fn clear_environment(&mut self) {
        self.machines.clear();
        self.accounts.clear();
    }
}

/// A multi-pick chip's words: its name when nothing is picked, else the picks (`Mac, Bedrock`,
/// or `Mac +2` past two).
#[must_use]
pub(crate) fn chip_title(name: &str, picks: &[Pick]) -> String {
    let on: Vec<&Pick> = picks.iter().filter(|pick| pick.on).collect();
    match on.as_slice() {
        [] => name.to_owned(),
        [one] => one.label.clone(),
        [one, two] => format!("{}, {}", one.label, two.label),
        [first, rest @ ..] => format!("{} +{}", first.label, rest.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(worker: Who, machine: &str, account: Option<&str>) -> Facts {
        Facts {
            worker,
            machine: Some(machine.to_owned()),
            account: account.map(str::to_owned),
            place: None,
        }
    }

    fn rows() -> Vec<Facts> {
        // The second runner goes by the place the server names it.
        let placed = |mut facts: Facts| {
            facts.place = Some("Mac".to_owned());
            facts
        };
        vec![
            placed(row(Who::Direct, "runner-b", Some("Personal"))),
            row(Who::Direct, "runner", Some("Work")),
            row(Who::Worker, "runner", Some("Work")),
            placed(row(Who::CodingApp, "runner-b", None)),
        ]
    }

    fn counts(picks: &[Pick]) -> Vec<(String, usize)> {
        picks.iter().map(|p| (p.label.clone(), p.count)).collect()
    }

    #[test]
    fn picks_in_one_menu_add_up_and_menus_narrow_each_other() {
        let mut filter = SessionFilter::default();
        filter.toggle(&Choice::Machine("runner-b".into()));
        filter.toggle(&Choice::Machine("runner".into()));
        assert_eq!(rows().iter().filter(|r| filter.keeps(r)).count(), 4);
        filter.toggle(&Choice::Account("Work".into()));
        assert_eq!(rows().iter().filter(|r| filter.keeps(r)).count(), 2);
        filter.toggle(&Choice::Worker(Who::Direct));
        assert_eq!(rows().iter().filter(|r| filter.keeps(r)).count(), 1);
        assert_eq!(filter.count(), 2);
    }

    #[test]
    fn a_menu_counts_its_picks_as_if_its_own_were_off() {
        let mut filter = SessionFilter::default();
        filter.toggle(&Choice::Worker(Who::Direct));
        let picks = filter.picks(&rows());
        assert_eq!(
            counts(&picks.workers),
            [
                ("Emberhut worker".to_owned(), 1),
                ("Your sessions".to_owned(), 2),
                ("External thread".to_owned(), 1)
            ]
        );
        assert!(picks.workers[1].on);
        assert_eq!(
            counts(&picks.environment),
            [
                ("Linux PC".to_owned(), 1),
                ("Mac".to_owned(), 1),
                ("Personal".to_owned(), 1),
                ("Work".to_owned(), 1)
            ],
            "the environment counts what the worker pick leaves"
        );
        assert_eq!(picks.environment[2].group, Some(Group::Account));
    }

    #[test]
    fn a_picked_choice_with_no_rows_stays_offered() {
        let mut filter = SessionFilter::default();
        filter.toggle(&Choice::Worker(Who::Lead));
        filter.toggle(&Choice::Machine("elsewhere".into()));
        let picks = filter.picks(&rows());
        assert!(
            picks
                .workers
                .iter()
                .any(|p| p.label == "Ultra lead" && p.count == 0)
        );
        assert_eq!(
            picks.environment[1].label, "elsewhere",
            "after the runner every account defaults to, by name"
        );
        filter.clear_workers();
        filter.clear_environment();
        assert!(filter.is_empty());
    }

    #[test]
    fn a_chip_names_what_is_picked() {
        let pick = |label: &str, on: bool| Pick {
            choice: Choice::Account(label.into()),
            label: label.into(),
            count: 1,
            group: None,
            on,
        };
        assert_eq!(chip_title("Worker", &[pick("A", false)]), "Worker");
        assert_eq!(
            chip_title("Environment", &[pick("Mac", true), pick("B", true)]),
            "Mac, B"
        );
        assert_eq!(
            chip_title(
                "Environment",
                &[pick("Mac", true), pick("B", true), pick("C", true)]
            ),
            "Mac +2"
        );
    }
}
