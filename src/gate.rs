//! The configurable quality gate: which results fail a check. Accepted
//! findings are in `baseline`, and which rules and levels fail by default in
//! `maturity`. Classification never depends on this policy.
use crate::{
    options::{CheckArgs, FailOn},
    schema::{FileResult, Finding, Gating, Report, Status, Strength},
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

/// Record each finding's measured precision and how the gate counts it, then
/// decide the gate for a complete run. Both read the maturity table, once
/// every level is final: grouping across files turns some findings into
/// notes. A finding in a preview language carries its language's own labels.
pub fn evaluate(report: &mut Report, args: &CheckArgs) {
    for file in &mut report.files {
        for finding in &mut file.findings {
            finding.precision =
                crate::maturity::precision_at(&file.path, &finding.rule, finding.strength);
            finding.preview =
                crate::maturity::preview_language(&file.path, &finding.rule).map(str::to_string);
            finding.gate = gating(finding, &file.path, args);
        }
    }
    // Notes are optional improvements; no gate counts them.
    let findings = report
        .files
        .iter()
        .flat_map(|f| &f.findings)
        .filter(|f| f.strength != Strength::Note);
    let baselined = findings.clone().filter(|f| f.baselined).count();
    let suppressed = findings
        .clone()
        .filter(|f| !f.baselined && f.suppressed.is_some())
        .count();
    let new: Vec<_> = findings.filter(|f| !f.accepted()).collect();
    let reasons = failures(report, &new, args);
    report.gate = report.complete.then_some(Gate {
        passed: reasons.is_empty(),
        reasons,
        new_findings: new.len(),
        baselined_findings: baselined,
        suppressed_findings: suppressed,
    });
}

/// How the gate counts a finding in `path`, at its rule's levels for that
/// path: none for notes and accepted findings. Consider counts every finding
/// and review only reviews; `mature` counts the rule's mature levels (a
/// custom question's own level), and a finding it leaves out is still being
/// measured. A preview language's findings are measured in that language
/// apart from the ten supported ones, so `mature` counts none of them.
fn gating(finding: &Finding, path: &Path, args: &CheckArgs) -> Option<Gating> {
    if finding.accepted() || finding.strength == Strength::Note {
        return None;
    }
    let levels = args.levels_at(&finding.rule, path);
    let mature = levels.contains(&FailOn::Mature);
    let counted = levels.contains(&FailOn::Consider)
        || (finding.strength == Strength::Review && levels.contains(&FailOn::Review))
        || (mature
            && crate::maturity::preview_language(path, &finding.rule).is_none()
            && args
                .mature_levels(&finding.rule)
                .contains(&finding.strength));
    Some(if counted {
        Gating::Fails
    } else if mature {
        Gating::Measuring
    } else {
        Gating::Advisory
    })
}

/// Why the gate fails: new findings at their rule's level, or undecided
/// results of a rule whose level includes `uncertain`.
fn failures(report: &Report, new: &[&Finding], args: &CheckArgs) -> Vec<String> {
    let mut reasons = Vec::new();
    let failing: Vec<_> = new.iter().filter(|f| f.fails_gate()).collect();
    let review = failing
        .iter()
        .filter(|f| f.strength == Strength::Review)
        .count();
    let consider = failing.len() - review;
    if review > 0 {
        reasons.push(crate::output::count(review, "new review finding"));
    }
    if consider > 0 {
        reasons.push(crate::output::count(consider, "new consider finding"));
    }
    let undecided = undecided_files(report, args).count();
    if undecided > 0 {
        reasons.push(format!(
            "{} with uncertain or needs-context results",
            crate::output::count(undecided, "file")
        ));
    }
    reasons
}

/// Whether undecided results of `rule` (a key) in `path` fail the gate: its
/// level there includes `uncertain`.
pub(crate) fn fails_undecided(args: &CheckArgs, rule: &str, path: &Path) -> bool {
    args.levels_at(rule, path).contains(&FailOn::Uncertain)
}

/// The files whose undecided results fail the gate: a rule whose level
/// includes `uncertain` left a unit undecided there, or the file needs
/// context while any rule's level does, since it has no dimensions.
pub(crate) fn undecided_files<'r>(
    report: &'r Report,
    args: &'r CheckArgs,
) -> impl Iterator<Item = &'r FileResult> + 'r {
    report.files.iter().filter(move |f| {
        let any_uncertain = args
            .rules
            .iter()
            .any(|rule| fails_undecided(args, rule, &f.path));
        f.dimensions.iter().any(|(rule, d)| {
            fails_undecided(args, rule, &f.path)
                && matches!(d.status, Status::Uncertain | Status::NeedsContext)
        }) || (any_uncertain && f.status == Status::NeedsContext)
    })
}

/// Apply the baseline, allow comments and the gate policy to a settled
/// report. Within an agent's turn, the baseline and allow comments accept
/// findings as they did when the turn began: accepting a finding is the
/// person's call, so what the agent wrote accepts nothing until the person
/// has seen it (the report's guards list it).
pub fn settle(root: &Path, report: &mut Report, args: &CheckArgs) -> Result<()> {
    let turn_start = args.turn_start();
    let ignored = match turn_start {
        Some(_) => crate::guards::added_allows(&report.guards),
        None => Default::default(),
    };
    crate::suppress::apply(root, report, &ignored);
    crate::baseline::apply(root, report, turn_start)?;
    evaluate(report, args);
    Ok(())
}
