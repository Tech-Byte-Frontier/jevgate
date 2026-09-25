//! The configurable quality gate: which results fail a check. Accepted
//! findings are in `baseline`. Classification never depends on this policy.
use crate::{
    options::{CheckArgs, FailOn},
    schema::{Finding, Report, Status, Strength},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Gate {
    pub passed: bool,
    /// Why the gate failed; empty when it passed or the run was incomplete.
    pub reasons: Vec<String>,
    pub new_findings: usize,
    pub baselined_findings: usize,
    /// Findings accepted by an inline `jevgate: allow` comment.
    #[serde(default)]
    pub suppressed_findings: usize,
}

/// Exit 0 when the gate passes, 1 when it fails, 2 when the run is incomplete.
pub fn exit_code(report: &Report) -> u8 {
    match &report.gate {
        _ if !report.complete => 2,
        Some(gate) if !gate.passed => 1,
        _ => 0,
    }
}

pub fn evaluate(report: &mut Report, args: &CheckArgs) {
    // Notes are optional improvements; no gate counts them.
    let findings = report
        .files
        .iter()
        .flat_map(|f| f.findings.iter().map(|finding| (f.path.as_path(), finding)))
        .filter(|(_, f)| f.strength != Strength::Note);
    let baselined = findings.clone().filter(|(_, f)| f.baselined).count();
    let suppressed = findings
        .clone()
        .filter(|(_, f)| !f.baselined && f.suppressed.is_some())
        .count();
    let new: Vec<_> = findings.filter(|(_, f)| !f.accepted()).collect();
    let reasons = failures(report, &new, args);
    report.gate = report.complete.then_some(Gate {
        passed: reasons.is_empty(),
        reasons,
        new_findings: new.len(),
        baselined_findings: baselined,
        suppressed_findings: suppressed,
    });
}

/// Whether a finding in `path` fails the gate: new, not a note, and at its
/// rule's level for that path. Consider is the lower bar, so it also fails
/// on review findings.
pub fn fails(finding: &Finding, path: &Path, args: &CheckArgs) -> bool {
    let levels = args.levels_at(&finding.rule, path);
    !finding.accepted()
        && finding.strength != Strength::Note
        && (levels.contains(&FailOn::Consider)
            || (finding.strength == Strength::Review && levels.contains(&FailOn::Review)))
}

/// Why the gate fails: new findings at their rule's level, or undecided
/// results of a rule whose level includes `uncertain`.
fn failures(report: &Report, new: &[(&Path, &Finding)], args: &CheckArgs) -> Vec<String> {
    let mut reasons = Vec::new();
    let failing: Vec<_> = new
        .iter()
        .filter(|(path, f)| fails(f, path, args))
        .collect();
    let review = failing
        .iter()
        .filter(|(_, f)| f.strength == Strength::Review)
        .count();
    let consider = failing.len() - review;
    if review > 0 {
        reasons.push(format!("{review} new review finding(s)"));
    }
    if consider > 0 {
        reasons.push(format!("{consider} new consider finding(s)"));
    }
    let uncertain =
        |rule: &str, path: &Path| args.levels_at(rule, path).contains(&FailOn::Uncertain);
    let undecided = report
        .files
        .iter()
        .filter(|f| {
            // A file that needs context has no dimensions; any rule that
            // fails on uncertain results for it counts it.
            let any_uncertain = args.rules.iter().any(|rule| uncertain(rule, &f.path));
            f.dimensions.iter().any(|(rule, d)| {
                uncertain(rule, &f.path)
                    && matches!(d.status, Status::Uncertain | Status::NeedsContext)
            }) || (any_uncertain && f.status == Status::NeedsContext)
        })
        .count();
    if undecided > 0 {
        reasons.push(format!(
            "{undecided} file(s) with uncertain or needs-context results"
        ));
    }
    reasons
}

/// Apply the baseline and the gate policy to a settled report.
pub fn settle(root: &Path, report: &mut Report, args: &CheckArgs) -> Result<()> {
    crate::suppress::apply(root, report);
    crate::baseline::apply(root, report)?;
    evaluate(report, args);
    Ok(())
}
