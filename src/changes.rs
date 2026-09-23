//! Finding lineage between snapshots, by fingerprint: introduced, persistent or resolved.
use crate::schema::{Change, Report, Status};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn change(
    rule: &str,
    path: &Path,
    fingerprint: &str,
    previous_generation: Option<u64>,
    state: &str,
    reason: &str,
) -> Change {
    Change {
        rule: rule.into(),
        path: path.into(),
        previous_path: None,
        previous_generation,
        state: state.into(),
        reason: reason.into(),
        fingerprint: fingerprint.into(),
    }
}

pub fn compare(previous: Option<&Report>, report: &mut Report) {
    report.changes = match previous {
        None => baseline(report),
        Some(previous) => lineage(previous, report),
    };
}

/// The first snapshot: every finding is a baseline, with an unknown start.
fn baseline(report: &Report) -> Vec<Change> {
    let mut changes = Vec::new();
    for file in &report.files {
        for finding in &file.findings {
            changes.push(change(
                &finding.rule,
                &file.path,
                &finding.fingerprint,
                None,
                "baseline",
                "First observed assessment; introduction time is unknown",
            ));
        }
    }
    changes
}

/// Current findings as persistent or introduced, then earlier ones as resolved.
fn lineage(previous: &Report, report: &Report) -> Vec<Change> {
    // Different questions or models can change findings without any code change.
    let comparable = previous.rubric_version == report.rubric_version
        && previous.requested_model == report.requested_model;
    let generation = Some(previous.generation);
    let before: BTreeMap<&str, (&Path, &str)> = previous
        .files
        .iter()
        .flat_map(|f| {
            f.findings
                .iter()
                .map(move |x| (x.fingerprint.as_str(), (f.path.as_path(), x.rule.as_str())))
        })
        .collect();
    let mut changes = current_changes(report, &before, comparable, generation);
    let current: BTreeSet<&str> = report
        .files
        .iter()
        .flat_map(|f| f.findings.iter().map(|x| x.fingerprint.as_str()))
        .collect();
    let judged = judged_paths(report);
    for (fingerprint, (path, rule)) in before {
        if current.contains(fingerprint) {
            continue;
        }
        let (state, reason) = if comparable && judged.contains(path) {
            (
                "resolved",
                "The finding no longer triggers; correctness is not certified",
            )
        } else {
            (
                "non-comparable",
                "The file was not judged in this snapshot, or rubric or model changed",
            )
        };
        changes.push(change(rule, path, fingerprint, generation, state, reason));
    }
    changes
}

/// Each current finding: persistent when seen before, else introduced (or not
/// comparable after a rubric or model change).
fn current_changes(
    report: &Report,
    before: &BTreeMap<&str, (&Path, &str)>,
    comparable: bool,
    generation: Option<u64>,
) -> Vec<Change> {
    let mut changes = Vec::new();
    for file in &report.files {
        for finding in &file.findings {
            let (state, reason) = if before.contains_key(finding.fingerprint.as_str()) {
                ("persistent", "The same finding remains")
            } else if comparable {
                ("introduced", "New since the previous snapshot")
            } else {
                (
                    "non-comparable",
                    "Rubric or model changed since the previous snapshot",
                )
            };
            changes.push(change(
                &finding.rule,
                &file.path,
                &finding.fingerprint,
                generation,
                state,
                reason,
            ));
        }
    }
    changes
}

/// Files this snapshot actually judged; only their missing findings are resolved.
fn judged_paths(report: &Report) -> BTreeSet<&Path> {
    report
        .files
        .iter()
        .filter(|f| !matches!(f.status, Status::Error | Status::Pending | Status::Skipped))
        .map(|f| f.path.as_path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Finding, Strength};

    fn finding(fingerprint: &str) -> Finding {
        Finding {
            rule: "maintainability/shared-logic".into(),
            strength: Strength::Review,
            line: 1,
            message: String::new(),
            action: String::new(),
            symbol: None,
            rule_version: "17".into(),
            concern_probability: 0.9,
            locations: Vec::new(),
            quote: None,
            category: None,
            values: Vec::new(),
            fingerprint: fingerprint.into(),
            rank: 1.0,
            baselined: false,
        }
    }

    #[test]
    fn fingerprints_track_introduced_persistent_and_resolved_findings() {
        let project = crate::tests::Project::new();
        project.write("a.rs", "fn a() {}\n");
        let options = crate::tests::args();
        let inputs = crate::inventory::collect(&options, &project.context(), &[]).unwrap();
        let mut old = crate::evaluate::snapshot(
            &inputs,
            &Default::default(),
            &options,
            crate::evaluate::SnapshotContext {
                root: &project.0,
                generation: 1,
                requests: 0,
            },
        );
        old.files[0].status = Status::Review;
        old.files[0].findings = vec![finding("kept"), finding("fixed")];
        let mut new = old.clone();
        new.generation = 2;
        new.files[0].findings = vec![finding("kept"), finding("added")];
        compare(Some(&old), &mut new);
        let states: BTreeMap<_, _> = new
            .changes
            .iter()
            .map(|c| (c.fingerprint.as_str(), c.state.as_str()))
            .collect();
        assert_eq!(states["kept"], "persistent");
        assert_eq!(states["added"], "introduced");
        assert_eq!(states["fixed"], "resolved");
        new.files[0].status = Status::Error;
        compare(Some(&old), &mut new);
        assert!(
            new.changes
                .iter()
                .any(|c| c.fingerprint == "fixed" && c.state == "non-comparable")
        );
        compare(None, &mut new);
        assert!(new.changes.iter().all(|c| c.state == "baseline"));
    }
}
