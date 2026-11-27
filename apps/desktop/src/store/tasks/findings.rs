//! Findings (a scan's or a review's results, filed under it): whether each one's fix landed,
//! in words, and a scan's findings grouped by that, the open ones first.

use emberhut_api::areas::{FindingItem, FindingsSummary};
use emberhut_api::ids::TaskId;
use emberhut_api::tasks::{Issue, Landed};

use super::words::Tone;

/// The order findings are grouped in: what still needs work first.
pub(crate) const ORDER: [Landed; 5] = [
    Landed::NotLanded,
    Landed::Partly,
    Landed::Unchecked,
    Landed::Landed,
    Landed::Superseded,
];

/// A finding's state, `Not verified yet` when none was given.
#[must_use]
pub(crate) fn landed(finding: &Issue) -> Landed {
    finding.landed.clone().unwrap_or(Landed::Unchecked)
}

/// The state's name.
#[must_use]
pub(crate) fn name(landed: &Landed) -> &'static str {
    match landed {
        Landed::Landed => "Landed",
        Landed::Partly => "Partly landed",
        Landed::NotLanded => "Not landed",
        Landed::Superseded => "Superseded",
        _ => "Not verified yet",
    }
}

/// What the state means.
#[must_use]
pub(crate) fn meaning(landed: &Landed) -> &'static str {
    match landed {
        Landed::Landed => "Landed: the fix is in the code, checked",
        Landed::Partly => "Partly landed: some of the fix is in the code",
        Landed::NotLanded => "Not landed: checked, and the fix isn't in the code",
        Landed::Superseded => "Superseded: later work replaced it",
        _ => "Not verified yet: nobody has checked the code for it",
    }
}

/// The state's colour.
#[must_use]
pub(crate) fn tone(landed: &Landed) -> Tone {
    match landed {
        Landed::Landed => Tone::Success,
        Landed::Partly => Tone::Warn,
        Landed::NotLanded => Tone::Danger,
        _ => Tone::Muted,
    }
}

/// `findings` grouped by state in [`ORDER`], groups with none left out.
#[must_use]
pub(crate) fn grouped(findings: &[Issue]) -> Vec<(Landed, Vec<&Issue>)> {
    ORDER
        .iter()
        .map(|state| {
            let rows: Vec<&Issue> = findings
                .iter()
                .filter(|finding| landed(finding) == *state)
                .collect();
            (state.clone(), rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

/// An area's findings grouped by state in [`ORDER`]: the open ones it lists (not landed,
/// partly, unchecked) and how many of each state it counts, groups with none left out. The
/// landed and superseded ones are counted, not listed.
#[must_use]
pub(crate) fn area_groups(summary: &FindingsSummary) -> Vec<(Landed, Vec<&FindingItem>, u32)> {
    ORDER
        .iter()
        .map(|state| {
            let rows: Vec<&FindingItem> = summary
                .unlanded
                .iter()
                .filter(|finding| finding.landed == *state)
                .collect();
            let counted = match state {
                Landed::Landed => summary.landed,
                Landed::Partly => summary.partly,
                Landed::NotLanded => summary.not_landed,
                Landed::Superseded => summary.superseded,
                _ => summary.unchecked,
            };
            let count = counted.max(u32::try_from(rows.len()).unwrap_or(u32::MAX));
            (state.clone(), rows, count)
        })
        .filter(|(_, _, count)| *count > 0)
        .collect()
}

/// The scans an area's listed findings came from, each once in the order first met, with its
/// title: the scan picker's choices (shown with more than one).
#[must_use]
pub(crate) fn scans(summary: &FindingsSummary) -> Vec<(TaskId, &str)> {
    let mut scans: Vec<(TaskId, &str)> = Vec::new();
    for finding in &summary.unlanded {
        if !scans.iter().any(|(scan, _)| *scan == finding.scan) {
            scans.push((finding.scan, finding.scan_title.as_str()));
        }
    }
    scans
}

/// [`area_groups`], narrowed to the findings of `scan` when one is picked: then each state
/// counts the listed ones of that scan only (the counted-only states have none).
#[must_use]
pub(crate) fn area_groups_of(
    summary: &FindingsSummary,
    scan: Option<TaskId>,
) -> Vec<(Landed, Vec<&FindingItem>, u32)> {
    let Some(scan) = scan else {
        return area_groups(summary);
    };
    ORDER
        .iter()
        .map(|state| {
            let rows: Vec<&FindingItem> = summary
                .unlanded
                .iter()
                .filter(|finding| finding.landed == *state && finding.scan == scan)
                .collect();
            let count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
            (state.clone(), rows, count)
        })
        .filter(|(_, _, count)| *count > 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::issue;

    #[test]
    fn findings_group_by_whether_they_landed_open_ones_first() {
        let findings = [
            issue(1, "done", json!({"kind": "finding", "landed": "landed"})),
            issue(
                2,
                "todo",
                json!({"kind": "finding", "landed": "not_landed"}),
            ),
            issue(3, "todo", json!({"kind": "finding"})),
            issue(4, "todo", json!({"kind": "finding", "landed": "landed"})),
        ];
        let groups: Vec<(&str, Vec<i64>)> = grouped(&findings)
            .iter()
            .map(|(state, rows)| (name(state), rows.iter().map(|f| f.id.get()).collect()))
            .collect();
        assert_eq!(
            groups,
            [
                ("Not landed", vec![2]),
                ("Not verified yet", vec![3]),
                ("Landed", vec![1, 4])
            ]
        );
        assert_eq!(
            meaning(&Landed::Superseded),
            "Superseded: later work replaced it"
        );
        assert_eq!(tone(&Landed::Partly), Tone::Warn);
    }

    #[test]
    fn an_areas_findings_list_the_open_ones_and_count_the_rest() {
        let summary: FindingsSummary = serde_json::from_value(json!({
            "total": 6, "landed": 3, "partly": 0, "notLanded": 1, "superseded": 0,
            "unchecked": 2, "open": 3,
            "unlanded": [
                {"id": 31, "title": "A gap", "landed": "not_landed", "scan": 30,
                 "scanTitle": "A scan", "fixedBy": 40, "fixedByStatus": "waiting"},
                {"id": 32, "title": "Maybe", "landed": "unchecked", "scan": 30,
                 "scanTitle": "A scan"},
                {"id": 33, "title": "Also maybe", "landed": "unchecked", "scan": 30,
                 "scanTitle": "A scan"}
            ]
        }))
        .unwrap();
        let groups: Vec<(&str, Vec<i64>, u32)> = area_groups(&summary)
            .iter()
            .map(|(state, rows, n)| (name(state), rows.iter().map(|f| f.id.get()).collect(), *n))
            .collect();
        assert_eq!(
            groups,
            [
                ("Not landed", vec![31], 1),
                ("Not verified yet", vec![32, 33], 2),
                ("Landed", vec![], 3)
            ]
        );
    }

    #[test]
    fn the_scan_picker_narrows_an_areas_findings_to_one_scan() {
        let summary: FindingsSummary = serde_json::from_value(json!({
            "total": 5, "landed": 2, "partly": 0, "notLanded": 2, "superseded": 0,
            "unchecked": 1, "open": 3,
            "unlanded": [
                {"id": 31, "title": "A gap", "landed": "not_landed", "scan": 30,
                 "scanTitle": "First scan"},
                {"id": 41, "title": "Another gap", "landed": "not_landed", "scan": 40,
                 "scanTitle": "Second scan"},
                {"id": 32, "title": "Maybe", "landed": "unchecked", "scan": 30,
                 "scanTitle": "First scan"}
            ]
        }))
        .unwrap();
        assert_eq!(
            scans(&summary),
            [
                (TaskId::new(30), "First scan"),
                (TaskId::new(40), "Second scan")
            ]
        );
        let of = |scan: Option<i64>| -> Vec<(&str, Vec<i64>, u32)> {
            area_groups_of(&summary, scan.map(TaskId::new))
                .iter()
                .map(|(state, rows, n)| {
                    (name(state), rows.iter().map(|f| f.id.get()).collect(), *n)
                })
                .collect()
        };
        assert_eq!(
            of(Some(40)),
            [("Not landed", vec![41], 1)],
            "the counted-only states have none of one scan"
        );
        assert_eq!(
            of(None)[2],
            ("Landed", vec![], 2),
            "every scan: the counts too"
        );
    }
}
