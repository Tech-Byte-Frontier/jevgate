//! The configurable quality gate: which results fail a check, and a baseline
//! of accepted findings. Classification never depends on this policy.
use crate::{
    options::{CheckArgs, FailOn},
    schema::{Finding, Report, Status, Strength},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

pub const BASELINE_FILE: &str = "jevgate-baseline.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Gate {
    pub passed: bool,
    /// Why the gate failed; empty when it passed or the run was incomplete.
    pub reasons: Vec<String>,
    pub new_findings: usize,
    pub baselined_findings: usize,
}

#[derive(Serialize, Deserialize)]
struct Baseline {
    version: u32,
    created_at: u64,
    findings: Vec<Accepted>,
}

#[derive(Serialize, Deserialize)]
struct Accepted {
    fingerprint: String,
    rule: String,
    path: std::path::PathBuf,
    message: String,
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
        .flat_map(|f| &f.findings)
        .filter(|f| f.strength != Strength::Note);
    let baselined = findings.clone().filter(|f| f.baselined).count();
    let new: Vec<_> = findings.filter(|f| !f.baselined).collect();
    let reasons = failures(report, &new, args);
    report.gate = report.complete.then_some(Gate {
        passed: reasons.is_empty(),
        reasons,
        new_findings: new.len(),
        baselined_findings: baselined,
    });
}

/// Whether a finding fails the gate: new, not a note, and at its rule's level.
/// Consider is the lower bar, so it also fails on review findings.
pub fn fails(finding: &Finding, args: &CheckArgs) -> bool {
    let levels = args.levels(&finding.rule);
    !finding.baselined
        && finding.strength != Strength::Note
        && (levels.contains(&FailOn::Consider)
            || (finding.strength == Strength::Review && levels.contains(&FailOn::Review)))
}

/// Why the gate fails: new findings at their rule's level, or undecided
/// results of a rule whose level includes `uncertain`.
fn failures(report: &Report, new: &[&Finding], args: &CheckArgs) -> Vec<String> {
    let mut reasons = Vec::new();
    let failing: Vec<&&Finding> = new.iter().filter(|f| fails(f, args)).collect();
    let review = failing
        .iter()
        .filter(|f| f.strength == Strength::Review)
        .count();
    let consider = failing.len() - review;
    if review > 0 {
        reasons.push(format!("{review} new review finding(s)"));
    }
    if consider > 0 {
        reasons.push(format!("{consider} new consider finding(s)"));
    }
    let uncertain = |rule: &str| args.levels(rule).contains(&FailOn::Uncertain);
    let any_uncertain = args.fail_on.contains(&FailOn::Uncertain)
        || args
            .rule_fail_on
            .values()
            .any(|levels| levels.contains(&FailOn::Uncertain));
    let undecided = report
        .files
        .iter()
        .filter(|f| {
            f.dimensions.iter().any(|(rule, d)| {
                uncertain(rule) && matches!(d.status, Status::Uncertain | Status::NeedsContext)
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
    apply_baseline(root, report)?;
    evaluate(report, args);
    Ok(())
}

/// The committed baseline, when there is one.
fn read_baseline(root: &Path) -> Result<Option<Baseline>> {
    let path = root.join(BASELINE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let text = crate::inventory::read_source(&path, 16 * 1024 * 1024)
        .with_context(|| format!("Cannot read {BASELINE_FILE}"))?;
    let baseline: Baseline =
        serde_json::from_str(&text).with_context(|| format!("Invalid {BASELINE_FILE}"))?;
    ensure!(baseline.version == 1, "Unsupported {BASELINE_FILE} version");
    Ok(Some(baseline))
}

/// Mark findings whose fingerprints the baseline accepted.
pub fn apply_baseline(root: &Path, report: &mut Report) -> Result<()> {
    let Some(baseline) = read_baseline(root)? else {
        return Ok(());
    };
    let accepted: BTreeSet<&str> = baseline
        .findings
        .iter()
        .map(|f| f.fingerprint.as_str())
        .collect();
    for finding in report.files.iter_mut().flat_map(|f| &mut f.findings) {
        finding.baselined = accepted.contains(finding.fingerprint.as_str());
    }
    Ok(())
}

/// What `jevgate baseline` wrote: the file, the findings it accepted from the
/// last check, and the earlier entries it kept for files that check did not cover.
pub struct Written {
    pub path: std::path::PathBuf,
    pub accepted: usize,
    pub kept: usize,
}

/// Accept every finding of the last complete check. No source is read or sent.
/// With `merge`, earlier entries stay for files the check did not cover, such
/// as unchanged files of a `--base` run; entries for checked or deleted files
/// are replaced by what the check found.
pub fn write_baseline(root: &Path, merge: bool) -> Result<Written> {
    let report = crate::storage::read_latest(root)
        .context("No compatible .jevgate/latest.json; run jevgate check first")?;
    ensure!(
        report.complete && !report.dry_run,
        "The last check was incomplete; rerun it before writing a baseline"
    );
    let mut findings: Vec<Accepted> = report
        .files
        .iter()
        .flat_map(|file| {
            file.findings.iter().map(|f| Accepted {
                fingerprint: f.fingerprint.clone(),
                rule: f.rule.clone(),
                path: file.path.clone(),
                message: f.message.clone(),
            })
        })
        .collect();
    let accepted = findings.len();
    let mut kept = 0;
    if merge && let Some(previous) = read_baseline(root)? {
        let covered: BTreeSet<&Path> = report
            .files
            .iter()
            .map(|f| f.path.as_path())
            .chain(report.deleted_files.iter().map(|p| p.as_path()))
            .collect();
        let earlier: Vec<Accepted> = previous
            .findings
            .into_iter()
            .filter(|f| !covered.contains(f.path.as_path()))
            .collect();
        kept = earlier.len();
        findings.extend(earlier);
    }
    findings.sort_by(|a, b| (&a.path, &a.fingerprint).cmp(&(&b.path, &b.fingerprint)));
    findings.dedup_by(|a, b| a.fingerprint == b.fingerprint);
    let path = root.join(BASELINE_FILE);
    let baseline = Baseline {
        version: 1,
        created_at: crate::schema::now(),
        findings,
    };
    let mut bytes = serde_json::to_vec_pretty(&baseline)?;
    bytes.push(b'\n');
    std::fs::write(&path, bytes).with_context(|| format!("Cannot write {}", path.display()))?;
    Ok(Written {
        path,
        accepted,
        kept,
    })
}
