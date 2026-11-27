//! A coding-thread app the test scripts: its machines, their threads and items, and what the
//! server read and sent.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a poisoned lock fails the test"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use emberhut_core::thread::ActivityItem;
use emberhut_server::services::coding_app::{
    CodingAppError, CodingThreadApp, ListedThread, ReadAsk, Sent, ThreadPage,
};
use futures_util::future::BoxFuture;

/// One scripted thread.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeThread {
    pub(crate) listed: Option<ListedThread>,
    pub(crate) status: Option<String>,
    pub(crate) updated_at: Option<String>,
    pub(crate) items: Vec<ActivityItem>,
}

#[derive(Debug, Default)]
struct Script {
    machines: Vec<String>,
    down: BTreeMap<String, String>,
    threads: BTreeMap<(String, String), FakeThread>,
    reads: Vec<(String, String, ReadAsk)>,
    sent: Vec<(String, String, String)>,
    refuse_sends: BTreeSet<String>,
}

/// The fake app. Every machine named answers unless it is [`Self::take_down`].
#[derive(Debug, Default)]
pub(crate) struct FakeCodingApp {
    script: Mutex<Script>,
}

/// An item of `kind` saying `text`.
pub(crate) fn item(kind: &str, text: &str) -> ActivityItem {
    ActivityItem {
        kind: kind.to_owned(),
        title: None,
        text: text.to_owned(),
        at: Some("2026-11-24T21:00:00.000Z".to_owned()),
    }
}

impl FakeCodingApp {
    /// The app is on these machines.
    pub(crate) fn on(&self, machines: &[&str]) {
        self.script.lock().unwrap().machines = machines.iter().map(|&m| m.to_owned()).collect();
    }

    /// `thread` on `machine` is listed as given, with these items.
    pub(crate) fn thread(&self, machine: &str, thread: ListedThread, items: Vec<ActivityItem>) {
        let fake = FakeThread {
            status: Some(thread.status.clone()),
            updated_at: Some(thread.updated_at.clone()),
            listed: Some(thread.clone()),
            items,
        };
        self.script
            .lock()
            .unwrap()
            .threads
            .insert((machine.to_owned(), thread.id), fake);
    }

    /// `thread` on `machine` can be read, but isn't listed.
    pub(crate) fn unlisted(&self, machine: &str, id: &str, fake: FakeThread) {
        self.script
            .lock()
            .unwrap()
            .threads
            .insert((machine.to_owned(), id.to_owned()), fake);
    }

    /// `thread` on `machine` is gone.
    pub(crate) fn forget(&self, machine: &str, id: &str) {
        self.script
            .lock()
            .unwrap()
            .threads
            .remove(&(machine.to_owned(), id.to_owned()));
    }

    /// `machine` can't be reached, for `why`.
    pub(crate) fn take_down(&self, machine: &str, why: &str) {
        self.script
            .lock()
            .unwrap()
            .down
            .insert(machine.to_owned(), why.to_owned());
    }

    /// Sends to `thread` are refused as an unknown thread.
    pub(crate) fn refuse_sends_to(&self, thread: &str) {
        self.script
            .lock()
            .unwrap()
            .refuse_sends
            .insert(thread.to_owned());
    }

    /// Every read so far: machine, thread and what was asked.
    pub(crate) fn reads(&self) -> Vec<(String, String, ReadAsk)> {
        self.script.lock().unwrap().reads.clone()
    }

    /// Every message sent so far: machine, thread and message.
    pub(crate) fn sent(&self) -> Vec<(String, String, String)> {
        self.script.lock().unwrap().sent.clone()
    }

    fn reachable(&self, machine: &str) -> Result<(), CodingAppError> {
        let script = self.script.lock().unwrap();
        if !script.machines.iter().any(|m| m == machine) {
            return Err(CodingAppError::UnknownMachine);
        }
        match script.down.get(machine) {
            Some(why) => Err(CodingAppError::Unreachable {
                machine: machine.to_owned(),
                reason: why.clone(),
            }),
            None => Ok(()),
        }
    }
}

impl CodingThreadApp for FakeCodingApp {
    fn machines(&self) -> Vec<String> {
        self.script.lock().unwrap().machines.clone()
    }

    fn list<'a>(
        &'a self,
        machine: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ListedThread>, CodingAppError>> {
        Box::pin(async move {
            self.reachable(machine)?;
            Ok(self
                .script
                .lock()
                .unwrap()
                .threads
                .iter()
                .filter(|((m, _), _)| m == machine)
                .filter_map(|(_, fake)| fake.listed.clone())
                .collect())
        })
    }

    fn read<'a>(
        &'a self,
        machine: &'a str,
        thread: &'a str,
        ask: ReadAsk,
    ) -> BoxFuture<'a, Result<ThreadPage, CodingAppError>> {
        Box::pin(async move {
            self.reachable(machine)?;
            let mut script = self.script.lock().unwrap();
            script
                .reads
                .push((machine.to_owned(), thread.to_owned(), ask));
            let fake = script
                .threads
                .get(&(machine.to_owned(), thread.to_owned()))
                .ok_or_else(|| CodingAppError::Refused {
                    machine: machine.to_owned(),
                    reason: "thread_not_found: No such thread".to_owned(),
                })?;
            let after = usize::try_from(ask.after.unwrap_or(0)).unwrap();
            Ok(ThreadPage {
                title: fake.listed.as_ref().map(|l| l.title.clone()),
                status: fake.status.clone(),
                updated_at: fake.updated_at.clone(),
                item_count: u64::try_from(fake.items.len()).unwrap(),
                items: fake
                    .items
                    .iter()
                    .skip(after)
                    .take(ask.limit)
                    .cloned()
                    .map(|mut item| {
                        item.text = item.text.chars().take(ask.max_chars).collect();
                        item
                    })
                    .collect(),
            })
        })
    }

    fn send<'a>(
        &'a self,
        machine: &'a str,
        thread: &'a str,
        message: &'a str,
    ) -> BoxFuture<'a, Result<Sent, CodingAppError>> {
        Box::pin(async move {
            self.reachable(machine)?;
            let mut script = self.script.lock().unwrap();
            if script.refuse_sends.contains(thread) {
                return Err(CodingAppError::Refused {
                    machine: machine.to_owned(),
                    reason: "thread_not_found: No such thread".to_owned(),
                });
            }
            script
                .sent
                .push((machine.to_owned(), thread.to_owned(), message.to_owned()));
            Ok(Sent {
                run_id: Some(format!("run-{}", script.sent.len())),
                queued: true,
            })
        })
    }
}
