//! What each runner machine has left: every runner samples its machine every 20 seconds and
//! posts it (`POST /api/runner/machine`); the server keeps the latest sample per machine and a
//! day of 5-minute points (each the worst of its samples), and shows them in the Machines view
//! (`GET /api/machines`) and the assistant's `machines` tool. A machine that is offline or too
//! full takes no new session ([`MachineService::can_place`]).
//!
//! The rules (warnings, states, words, points) are `emberhut-core`'s [`emberhut_core::machine`].

use std::collections::BTreeMap;
use std::sync::Arc;

use emberhut_api as api;
use emberhut_core::Clock;
use emberhut_core::machine::{
    BatterySample, CapSample, CpuSample, DiskSample, GpuSample, KEEP_MS, MachineState,
    MemorySample, Point, RunnerSample, Sample, Seen, Thresholds, bucket_of, can_place, state_of,
    status, summary, warnings,
};
use emberhut_db::{DbError, MachineStore, StoredSample, ThreadStore, UsageStore};
use jiff::Timestamp;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::convert::timestamp_to_api;
use crate::services::runners::{RunnerService, text};

/// How often old points are dropped, in milliseconds.
const PRUNE_EVERY: i64 = 600_000;

/// Why a sample wasn't taken.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MachineError {
    /// Not a sample of the runner that sent it.
    #[error("{0}")]
    Refused(String),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// The machines' samples, states and history.
#[derive(Clone, Debug)]
pub struct MachineService {
    store: MachineStore,
    threads: ThreadStore,
    usage: UsageStore,
    runners: RunnerService,
    clock: Arc<dyn Clock>,
    started: Timestamp,
    limits: Thresholds,
    pruned: Arc<std::sync::atomic::AtomicI64>,
}

/// A machine's latest sample, read.
struct Latest {
    at: Timestamp,
    json: Value,
    sample: Sample,
}

impl MachineService {
    /// The service over `store`, the threads and accounts it shows with each machine, and the
    /// runners; the server counts as started now.
    #[must_use]
    pub fn new(
        store: MachineStore,
        threads: ThreadStore,
        usage: UsageStore,
        runners: RunnerService,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let started = clock.now();
        Self {
            store,
            threads,
            usage,
            runners,
            clock,
            started,
            limits: Thresholds::default(),
            pruned: Arc::default(),
        }
    }

    /// The limits the states and warnings are judged by.
    #[must_use]
    pub fn thresholds(&self) -> Thresholds {
        self.limits
    }

    /// `machine`'s runner's sample (`body`): kept as its latest and folded into its 5-minute
    /// point.
    ///
    /// # Errors
    ///
    /// [`MachineError::Refused`] when it isn't an object, or names another runner;
    /// [`MachineError::Database`].
    pub async fn take(&self, machine: &str, body: &Value) -> Result<(), MachineError> {
        if !body.is_object() {
            return Err(MachineError::Refused("not a sample".to_owned()));
        }
        let named = text(body, "machine").unwrap_or_default();
        if named != machine || !self.runners.is_runner(&named) {
            return Err(MachineError::Refused(format!("no runner called {named}")));
        }
        self.runners.heard(machine);
        let now = self.clock.now();
        let at_ms = now.as_millisecond();
        let bucket = bucket_of(at_ms);
        let point = Point::of(&sample_of(body), at_ms);
        let prune = (at_ms - self.pruned.load(std::sync::atomic::Ordering::Relaxed) > PRUNE_EVERY)
            .then(|| {
                self.pruned
                    .store(at_ms, std::sync::atomic::Ordering::Relaxed);
                at_ms - KEEP_MS
            });
        let stored = StoredSample {
            machine: machine.to_owned(),
            at: now,
            data: body.to_string(),
        };
        self.store
            .take(
                &stored,
                bucket,
                |was| {
                    let point = match was
                        .and_then(|was| serde_json::from_str::<api::machines::Point>(was).ok())
                    {
                        Some(was) => point_from_api(&was, bucket).worst(point),
                        None => point,
                    };
                    serde_json::to_string(&point_to_api(&point)).unwrap_or_default()
                },
                prune,
            )
            .await?;
        Ok(())
    }

    async fn latest(&self) -> Result<BTreeMap<String, Latest>, DbError> {
        Ok(self
            .store
            .latest()
            .await?
            .into_iter()
            .filter_map(|row| {
                let json: Value = serde_json::from_str(&row.data).ok()?;
                let sample = sample_of(&json);
                Some((
                    row.machine,
                    Latest {
                        at: row.at,
                        json,
                        sample,
                    },
                ))
            })
            .collect())
    }

    fn state(&self, latest: Option<&Latest>) -> (MachineState, Option<f64>) {
        let seen = latest.map(|latest| Seen {
            sample: &latest.sample,
            at: latest.at,
        });
        state_of(seen.as_ref(), self.started, self.clock.now(), &self.limits)
    }

    /// Whether a new session may start on `machine`, and why not.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn can_place(&self, machine: &str) -> Result<Result<(), String>, DbError> {
        let latest = self.latest().await?;
        let latest = latest.get(machine);
        let (state, age) = self.state(latest);
        Ok(can_place(
            state,
            age,
            latest.map(|latest| &latest.sample),
            &self.limits,
        ))
    }

    /// One line per runner machine (only `only`'s when given), for the assistant.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn summaries(&self, only: Option<&str>) -> Result<Vec<String>, DbError> {
        let latest = self.latest().await?;
        Ok(self
            .runners
            .machines()
            .into_iter()
            .filter(|machine| only.is_none_or(|only| only == machine))
            .map(|machine| {
                let seen = latest.get(&machine);
                let (state, age) = self.state(seen);
                summary(
                    &self.runners.place(&machine),
                    &machine,
                    state,
                    age,
                    seen.map(|latest| &latest.sample),
                    &self.limits,
                )
            })
            .collect())
    }

    /// The Machines view: every runner machine's state, figures, warnings, sessions and (with
    /// `history`) its last day.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    #[expect(
        clippy::too_many_lines,
        reason = "one field per figure of the contract's machine"
    )]
    pub async fn view(&self, history: bool) -> Result<api::machines::MachinesPage, DbError> {
        let latest = self.latest().await?;
        let accounts = self.usage.all().await?;
        let now = self.clock.now();
        let mut machines = Vec::new();
        for machine in self.runners.machines() {
            let seen = latest.get(&machine);
            let (state, age) = self.state(seen);
            let placed = can_place(state, age, seen.map(|latest| &latest.sample), &self.limits);
            let empty = Value::Null;
            let json = seen.map_or(&empty, |latest| &latest.json);
            let threads = self.threads.on_machine(&machine).await?;
            let sessions = json
                .get("sessions")
                .and_then(Value::as_array)
                .map_or_default(|rows| {
                    rows.iter()
                        .filter_map(|row| {
                            let id = text(row, "id")?;
                            let thread = threads.iter().find(|thread| thread.id == id);
                            let account = text(row, "account")
                                .or_else(|| thread.and_then(|thread| thread.account.clone()));
                            let known = account.as_deref().and_then(|account| {
                                accounts
                                    .iter()
                                    .find(|a| a.machine == machine && a.id == account)
                            });
                            Some(api::machines::MachineSession {
                                id: id.into(),
                                title: thread
                                    .map(|thread| thread.title.clone())
                                    .or_else(|| text(row, "title")),
                                status: thread
                                    .map(|thread| thread.status.as_str().to_owned())
                                    .or_else(|| text(row, "status")),
                                running: row.get("running").and_then(Value::as_bool) == Some(true),
                                doing: thread.and_then(|thread| thread.doing.clone()),
                                account_name: known
                                    .map(|a| a.name.clone())
                                    .or_else(|| account.clone()),
                                account,
                                used5h: known.and_then(|a| a.used5h),
                                used7d: known.and_then(|a| a.used7d),
                                limited_until: known
                                    .and_then(|a| a.limited_until)
                                    .map(timestamp_to_api),
                            })
                        })
                        .collect()
                });
            let history = if history {
                Some(
                    self.store
                        .points_since(&machine, now.as_millisecond() - KEEP_MS)
                        .await?
                        .iter()
                        .filter_map(|point| serde_json::from_str(point).ok())
                        .collect(),
                )
            } else {
                None
            };
            machines.push(api::machines::Machine {
                id: machine.clone().into(),
                name: self.runners.place(&machine),
                host: text(json, "host"),
                platform: text(json, "platform"),
                state: api::machines::MachineState::from(state.as_str()),
                status: status(state, age),
                seen: seen.map(|latest| timestamp_to_api(latest.at)),
                age: age.map(f64::round),
                placeable: Some(placed.is_ok()),
                skip_why: placed.err(),
                warnings: seen.map_or_default(|latest| {
                    warnings(&latest.sample, &self.limits)
                        .into_iter()
                        .map(|warning| api::machines::MachineWarning {
                            level: api::machines::WarningLevel::from(warning.level.as_str()),
                            text: warning.text,
                        })
                        .collect()
                }),
                cpu: field(json, "cpu"),
                memory: field(json, "memory"),
                gpus: field(json, "gpus").unwrap_or_default(),
                disks: field(json, "disks").unwrap_or_default(),
                battery: field(json, "battery"),
                runner: json
                    .get("runner")
                    .filter(|r| r.is_object())
                    .map(runner_facts),
                sessions,
                history,
            });
        }
        let limits = &self.limits;
        Ok(api::machines::MachinesPage {
            at: timestamp_to_api(now),
            thresholds: Some(api::machines::Thresholds {
                min_available_gb: limits.min_available_gb,
                min_free_disk_gb: limits.min_free_disk_gb,
                max_load_per_core: limits.max_load_per_core,
                alert_disk_gb: limits.alert_disk_gb,
                warn_disk_pct: limits.warn_disk_pct,
                min_cap_room_gb: limits.min_cap_room_gb,
                stale_seconds: limits.stale_seconds,
                offline_seconds: limits.offline_seconds,
            }),
            machines,
        })
    }
}

/// `key` of `json` read as `T`, when it is there and reads.
fn field<T: DeserializeOwned>(json: &Value, key: &str) -> Option<T> {
    json.get(key)
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// A finite number at `key`.
fn num(json: &Value, key: &str) -> Option<f64> {
    json.get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}

/// The runner's own figures, as the view shows them (its cap only when it has a limit).
fn runner_facts(runner: &Value) -> api::machines::RunnerFacts {
    let count = |key: &str| {
        runner
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    let cap = runner
        .get("cgroup")
        .filter(|cap| num(cap, "max").is_some() || num(cap, "high").is_some())
        .map(|cap| api::machines::Cap {
            name: text(cap, "name"),
            used: num(cap, "capUsed").or_else(|| num(cap, "current")),
            high: num(cap, "high"),
            max: num(cap, "max"),
            by: text(cap, "maxBy").or_else(|| text(cap, "highBy")),
        });
    api::machines::RunnerFacts {
        sessions: count("sessions"),
        queued: count("queued"),
        open: count("open"),
        claude: count("claude"),
        claude_rss: num(runner, "claudeRss"),
        rss: num(runner, "rss"),
        cap,
    }
}

/// The figures of a runner's sample the rules read.
pub(crate) fn sample_of(json: &Value) -> Sample {
    let list = |key: &str| {
        json.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    Sample {
        cpu: json
            .get("cpu")
            .filter(|cpu| cpu.is_object())
            .map(|cpu| CpuSample {
                cores: num(cpu, "cores"),
                load1: num(cpu, "load1"),
                load5: num(cpu, "load5"),
                usage: num(cpu, "usage"),
            }),
        memory: json
            .get("memory")
            .filter(|memory| memory.is_object())
            .map(|memory| MemorySample {
                total: num(memory, "total"),
                available: num(memory, "available"),
                swap_total: num(memory, "swapTotal"),
                swap_used: num(memory, "swapUsed"),
                pressure: text(memory, "pressure"),
            }),
        gpus: list("gpus")
            .iter()
            .map(|gpu| GpuSample {
                name: text(gpu, "name"),
                util: num(gpu, "util"),
                mem_used: num(gpu, "memUsed"),
                mem_total: num(gpu, "memTotal"),
                cores: num(gpu, "cores"),
                shared: gpu.get("shared").and_then(Value::as_bool) == Some(true),
                procs: gpu
                    .get("procs")
                    .and_then(Value::as_array)
                    .map_or_default(|procs| {
                        procs
                            .iter()
                            .map(|proc| {
                                (
                                    text(proc, "name").unwrap_or_default(),
                                    text(proc, "unit"),
                                    num(proc, "mem").unwrap_or(0.0),
                                )
                            })
                            .collect()
                    }),
            })
            .collect(),
        disks: list("disks")
            .iter()
            .map(|disk| DiskSample {
                path: text(disk, "path"),
                total: num(disk, "total"),
                free: num(disk, "free"),
            })
            .collect(),
        battery: json
            .get("battery")
            .filter(|battery| battery.is_object())
            .map(|battery| BatterySample {
                percent: num(battery, "percent"),
                plugged: battery.get("plugged").and_then(Value::as_bool) == Some(true),
                state: text(battery, "state"),
                minutes_left: num(battery, "minutesLeft"),
            }),
        runner: json
            .get("runner")
            .filter(|r| r.is_object())
            .map(|runner| RunnerSample {
                sessions: num(runner, "sessions"),
                claude: num(runner, "claude"),
                cap: runner
                    .get("cgroup")
                    .filter(|cap| cap.is_object())
                    .map(|cap| CapSample {
                        name: text(cap, "name"),
                        max: num(cap, "max"),
                        high: num(cap, "high"),
                        cap_used: num(cap, "capUsed"),
                        current: num(cap, "current"),
                        max_by: text(cap, "maxBy"),
                        high_by: text(cap, "highBy"),
                    }),
            }),
    }
}

/// A point as the contract writes it (`at`: the start of its 5 minutes).
fn point_to_api(point: &Point) -> api::machines::Point {
    api::machines::Point {
        at: Timestamp::from_millisecond(point.bucket)
            .ok()
            .map(timestamp_to_api),
        cpu: point.cpu,
        load: point.load,
        mem_avail: point.mem_avail,
        gpu: point.gpu,
        vram: point.vram,
        disk_free: point.disk_free,
    }
}

/// A stored point, back in the rules' terms.
fn point_from_api(point: &api::machines::Point, bucket: i64) -> Point {
    Point {
        bucket,
        cpu: point.cpu,
        load: point.load,
        mem_avail: point.mem_avail,
        gpu: point.gpu,
        vram: point.vram,
        disk_free: point.disk_free,
    }
}
