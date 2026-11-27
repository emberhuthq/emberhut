//! The tree of tasks: who is under whom, and a parent's subtasks at a glance.

use std::collections::{BTreeMap, BTreeSet};

use super::{Status, TaskId};

/// A task as the tree needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Light {
    /// The task.
    pub id: TaskId,
    /// Its parent.
    pub parent: Option<TaskId>,
    /// Where it is.
    pub status: Status,
    /// Its count so far.
    pub progress: Option<i64>,
    /// What its count goes to.
    pub total: Option<i64>,
    /// What it counts.
    pub unit: Option<String>,
    /// It is a finding.
    pub finding: bool,
}

/// A parent's subtasks at a glance: how many are done (canceled ones don't count), which wait
/// on the person at any depth, and their counts added up (only when every subtask has one,
/// else the sum misleads); `left`: what the open ones still need, when each has a count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rollup {
    /// Subtasks done.
    pub done: u32,
    /// Subtasks, canceled ones left out.
    pub total: u32,
    /// Tasks under it waiting on the person, at any depth.
    pub waiting: Vec<TaskId>,
    /// The counts added up.
    pub progress: Option<i64>,
    /// What they go to.
    pub of: Option<i64>,
    /// What the open ones still need.
    pub left: Option<i64>,
    /// What they count.
    pub unit: Option<String>,
}

/// Every task's place in the tree.
#[derive(Clone, Debug, Default)]
pub struct Family {
    rows: BTreeMap<TaskId, Light>,
    kids: BTreeMap<TaskId, Vec<TaskId>>,
}

impl Family {
    /// The tree of `rows`.
    #[must_use]
    pub fn new(rows: impl IntoIterator<Item = Light>) -> Self {
        let mut family = Self::default();
        for row in rows {
            if let Some(parent) = row.parent {
                family.kids.entry(parent).or_default().push(row.id);
            }
            family.rows.insert(row.id, row);
        }
        for kids in family.kids.values_mut() {
            kids.sort_unstable();
        }
        family
    }

    /// The task `id`, if it is in the tree.
    #[must_use]
    pub fn get(&self, id: TaskId) -> Option<&Light> {
        self.rows.get(&id)
    }

    /// Its direct subtasks and findings, by id.
    #[must_use]
    pub fn kids(&self, id: TaskId) -> &[TaskId] {
        self.kids.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Its parents, nearest first (a loop already in the data stops the walk).
    #[must_use]
    pub fn ancestors(&self, id: TaskId) -> Vec<TaskId> {
        let mut up = Vec::new();
        let mut at = self.rows.get(&id).and_then(|row| row.parent);
        while let Some(parent) = at {
            if parent == id || up.contains(&parent) {
                break;
            }
            up.push(parent);
            at = self.rows.get(&parent).and_then(|row| row.parent);
        }
        up
    }

    /// Everything under it, at any depth (findings too), breadth first.
    #[must_use]
    pub fn descendants(&self, id: TaskId) -> Vec<TaskId> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::from([id]);
        let mut level = vec![id];
        while !level.is_empty() {
            let mut next = Vec::new();
            for at in level {
                for kid in self.kids(at) {
                    if seen.insert(*kid) {
                        out.push(*kid);
                        next.push(*kid);
                    }
                }
            }
            level = next;
        }
        out
    }

    /// Levels of tasks under it (0: none), findings left out.
    #[must_use]
    pub fn height(&self, id: TaskId) -> usize {
        let mut height = 0;
        let mut seen = BTreeSet::from([id]);
        let mut level = vec![id];
        loop {
            let next: Vec<TaskId> = level
                .iter()
                .flat_map(|at| self.kids(*at))
                .filter(|kid| self.rows.get(kid).is_some_and(|row| !row.finding))
                .filter(|kid| seen.insert(**kid))
                .copied()
                .collect();
            if next.is_empty() {
                return height;
            }
            height += 1;
            level = next;
        }
    }

    /// Its subtasks at a glance; none when it has no subtask (findings and canceled ones
    /// don't count).
    #[must_use]
    pub fn rollup(&self, id: TaskId) -> Option<Rollup> {
        let kids: Vec<&Light> = self
            .kids(id)
            .iter()
            .filter_map(|kid| self.rows.get(kid))
            .filter(|kid| !kid.finding && kid.status != Status::Canceled)
            .collect();
        if kids.is_empty() {
            return None;
        }
        let mut waiting: Vec<TaskId> = self
            .descendants(id)
            .into_iter()
            .filter_map(|at| self.rows.get(&at))
            .filter(|row| !row.finding && row.status == Status::Waiting)
            .map(|row| row.id)
            .collect();
        waiting.sort_unstable();
        let counted = |row: &Light| row.total.filter(|total| *total != 0);
        let did = |row: &Light| {
            let total = row.total.unwrap_or(0);
            if row.status == Status::Done {
                total
            } else {
                row.progress.unwrap_or(0).min(total)
            }
        };
        let every = kids.iter().all(|kid| counted(kid).is_some());
        let open: Vec<&&Light> = kids
            .iter()
            .filter(|kid| kid.status != Status::Done)
            .collect();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Some(Rollup {
            done: count(kids.len() - open.len()),
            total: count(kids.len()),
            waiting,
            progress: every.then(|| kids.iter().map(|kid| did(kid)).sum()),
            of: every.then(|| kids.iter().map(|kid| kid.total.unwrap_or(0)).sum()),
            left: (!open.is_empty() && open.iter().all(|kid| counted(kid).is_some())).then(|| {
                open.iter()
                    .map(|kid| kid.total.unwrap_or(0) - did(kid))
                    .sum()
            }),
            unit: kids
                .iter()
                .find_map(|kid| kid.unit.clone().filter(|unit| !unit.is_empty())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(id: i64, parent: Option<i64>, status: Status) -> Light {
        Light {
            id: TaskId::new(id),
            parent: parent.map(TaskId::new),
            status,
            progress: None,
            total: None,
            unit: None,
            finding: false,
        }
    }

    #[test]
    fn a_parent_rolls_up_its_subtasks() {
        let mut a = light(2, Some(1), Status::Done);
        a.total = Some(10);
        a.unit = Some("episodes".to_owned());
        let mut b = light(3, Some(1), Status::Working);
        b.total = Some(12);
        b.progress = Some(4);
        let family = Family::new([
            light(1, None, Status::Working),
            a,
            b,
            light(4, Some(1), Status::Canceled),
            light(5, Some(3), Status::Waiting),
        ]);
        let rollup = family.rollup(TaskId::new(1)).unwrap();
        assert_eq!((rollup.done, rollup.total), (1, 2));
        assert_eq!(rollup.waiting, [TaskId::new(5)]);
        assert_eq!(
            (rollup.progress, rollup.of, rollup.left),
            (Some(14), Some(22), Some(8))
        );
        assert_eq!(rollup.unit.as_deref(), Some("episodes"));
        assert!(family.rollup(TaskId::new(2)).is_none());
        assert_eq!(
            family.ancestors(TaskId::new(5)),
            [TaskId::new(3), TaskId::new(1)]
        );
        assert_eq!(
            family.descendants(TaskId::new(1)),
            [2, 3, 4, 5].map(TaskId::new)
        );
        assert_eq!(family.height(TaskId::new(1)), 2);
    }

    #[test]
    fn a_loop_in_the_data_stops_the_walk() {
        let family = Family::new([
            light(1, Some(2), Status::Todo),
            light(2, Some(1), Status::Todo),
        ]);
        assert_eq!(family.ancestors(TaskId::new(1)), [TaskId::new(2)]);
        assert_eq!(family.descendants(TaskId::new(1)), [TaskId::new(2)]);
    }
}
