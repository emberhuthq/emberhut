//! A machine's last 24 hours as two lines: free memory (of the total) and processor busy (load
//! per core where a point has no busy figure), each point placed by its time. A point without a
//! value breaks its line. Pure, so it is tested without drawing.

use emberhut_api::machines::Machine;
use jiff::Timestamp;

/// How far back the trend reaches.
const DAY_SECONDS: f64 = 86_400.;

/// A point of a line: `x` and `y` from 0 to 1; no `y` breaks the line there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dot {
    /// Where in the last 24 hours, 0 the start, 1 now.
    pub(crate) x: f64,
    /// Its value, 0 to 1.
    pub(crate) y: Option<f64>,
}

/// The two lines and the least free memory over the day.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Series {
    /// Free memory, as a share of the total.
    pub(crate) memory: Vec<Dot>,
    /// Processor busy.
    pub(crate) cpu: Vec<Dot>,
    /// The least memory that was free, in bytes.
    pub(crate) lowest: Option<f64>,
}

impl Series {
    /// `machine`'s last 24 hours up to `now`.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "seconds since the epoch are far inside f64's exact range"
    )]
    pub(crate) fn of(machine: &Machine, now: Timestamp) -> Self {
        let points = machine.history.as_deref().unwrap_or_default();
        let free = points.iter().filter_map(|point| point.mem_avail);
        let lowest = free.clone().reduce(f64::min);
        let total = machine
            .memory
            .as_ref()
            .and_then(|memory| memory.total)
            .or_else(|| free.reduce(f64::max))
            .unwrap_or(0.);
        let cores = machine.cpu.as_ref().and_then(|cpu| cpu.cores).unwrap_or(0.);
        let start = now.as_second() as f64 - DAY_SECONDS;
        let mut series = Self {
            lowest,
            ..Self::default()
        };
        for point in points {
            let Some(at) = point
                .at
                .as_ref()
                .and_then(|at| at.as_str().parse::<Timestamp>().ok())
            else {
                continue;
            };
            let x = (at.as_second() as f64 - start) / DAY_SECONDS;
            let memory = point
                .mem_avail
                .filter(|_| total > 0.)
                .map(|free| free / total);
            let busy = point
                .cpu
                .or_else(|| point.load.filter(|_| cores > 0.).map(|load| load / cores));
            series.memory.push(Dot { x, y: memory });
            series.cpu.push(Dot { x, y: busy });
        }
        series
    }
}

/// The runs of a line to draw: each a list of `(x, y)` from 0 to 1 with `y` upwards, split
/// where a point has no value.
#[must_use]
pub(crate) fn runs(dots: &[Dot]) -> Vec<Vec<(f64, f64)>> {
    let mut runs = Vec::new();
    let mut run = Vec::new();
    for dot in dots {
        match dot.y {
            Some(y) => run.push((dot.x.clamp(0., 1.), y.clamp(0., 1.))),
            None if !run.is_empty() => runs.push(std::mem::take(&mut run)),
            None => {}
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn machine(extra: serde_json::Value) -> Machine {
        let mut base = json!({"id": "a", "name": "pc", "state": "ok", "status": "fine"});
        if let (Some(base), serde_json::Value::Object(extra)) = (base.as_object_mut(), extra) {
            base.extend(extra);
        }
        serde_json::from_value(base).unwrap()
    }

    fn now() -> Timestamp {
        "2026-01-10T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn points_go_by_their_time_and_cpu_falls_back_to_load_per_core() {
        let machine = machine(json!({
            "memory": {"total": 100.0},
            "cpu": {"cores": 4.0},
            "history": [
                {"at": "2026-01-09T12:00:00Z", "cpu": 0.5, "memAvail": 40.0},
                {"at": "2026-01-10T00:00:00Z", "load": 2.0, "memAvail": 20.0},
                {"at": "2026-01-10T12:00:00Z"}
            ]
        }));
        let series = Series::of(&machine, now());
        let xs: Vec<f64> = series.cpu.iter().map(|dot| dot.x).collect();
        assert_eq!(xs, [0., 0.5, 1.]);
        assert_eq!(
            series.cpu.iter().map(|dot| dot.y).collect::<Vec<_>>(),
            [Some(0.5), Some(0.5), None]
        );
        assert_eq!(
            series.memory.iter().map(|dot| dot.y).collect::<Vec<_>>(),
            [Some(0.4), Some(0.2), None]
        );
        assert_eq!(series.lowest, Some(20.));
    }

    #[test]
    fn a_point_without_a_value_breaks_the_line() {
        let dot = |x, y| Dot { x, y };
        let runs = runs(&[
            dot(0., Some(0.1)),
            dot(0.1, Some(0.2)),
            dot(0.2, None),
            dot(0.3, Some(2.)),
        ]);
        assert_eq!(runs, [vec![(0., 0.1), (0.1, 0.2)], vec![(0.3, 1.)]]);
        assert_eq!(super::runs(&[dot(0., None)]), Vec::<Vec<(f64, f64)>>::new());
    }
}
