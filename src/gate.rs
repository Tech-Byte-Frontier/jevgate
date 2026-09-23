//! The configurable quality gate: which results fail a check, and a baseline
//! of accepted findings. Classification never depends on this policy.
use crate::{
    options::FailOn,
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

pub fn evaluate(report: &mut Report, fail_on: &[FailOn]) {
    let findings = report.files.iter().flat_map(|f| &f.findings);
    let baselined = findings.clone().filter(|f| f.baselined).count();
    let new: Vec<_> = findings.filter(|f| !f.baselined).collect();
    let reasons = failures(report, &new, fail_on);
    report.gate = report.complete.then_some(Gate {
        passed: reasons.is_empty(),
        reasons,
        new_findings: new.len(),
        baselined_findings: baselined,
    });
}

/// Why the gate fails: new findings at a configured level, or undecided files.
fn failures(report: &Report, new: &[&Finding], fail_on: &[FailOn]) -> Vec<String> {
    let mut reasons = Vec::new();
    let review = new
        .iter()
        .filter(|f| f.strength == Strength::Review)
        .count();
    let consider = new.len() - review;
    // Consider is the lower bar, so it also fails on review findings.
    let consider_bar = fail_on.contains(&FailOn::Consider);
    if (consider_bar || fail_on.contains(&FailOn::Review)) && review > 0 {
        reasons.push(format!("{review} new review finding(s)"));
    }
    if consider_bar && consider > 0 {
        reasons.push(format!("{consider} new consider finding(s)"));
    }
    if fail_on.contains(&FailOn::Uncertain) {
        let undecided = report
            .files
            .iter()
            .filter(|f| {
                f.dimensions
                    .values()
                    .any(|d| matches!(d.status, Status::Uncertain | Status::NeedsContext))
                    || f.status == Status::NeedsContext
            })
            .count();
        if undecided > 0 {
            reasons.push(format!(
                "{undecided} file(s) with uncertain or needs-context results"
            ));
        }
    }
    reasons
}

/// Apply the baseline and the gate policy to a settled report.
pub fn settle(root: &Path, report: &mut Report, fail_on: &[FailOn]) -> Result<()> {
    apply_baseline(root, report)?;
    evaluate(report, fail_on);
    Ok(())
}

/// Mark findings whose fingerprints the baseline accepted.
pub fn apply_baseline(root: &Path, report: &mut Report) -> Result<()> {
    let path = root.join(BASELINE_FILE);
    if !path.exists() {
        return Ok(());
    }
    let text = crate::inventory::read_source(&path, 16 * 1024 * 1024)
        .with_context(|| format!("Cannot read {BASELINE_FILE}"))?;
    let baseline: Baseline =
        serde_json::from_str(&text).with_context(|| format!("Invalid {BASELINE_FILE}"))?;
    ensure!(baseline.version == 1, "Unsupported {BASELINE_FILE} version");
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

/// Accept every finding of the last complete check. No source is read or sent.
pub fn write_baseline(root: &Path) -> Result<(std::path::PathBuf, usize)> {
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
    findings.sort_by(|a, b| (&a.path, &a.fingerprint).cmp(&(&b.path, &b.fingerprint)));
    findings.dedup_by(|a, b| a.fingerprint == b.fingerprint);
    let count = findings.len();
    let path = root.join(BASELINE_FILE);
    let baseline = Baseline {
        version: 1,
        created_at: crate::schema::now(),
        findings,
    };
    let mut bytes = serde_json::to_vec_pretty(&baseline)?;
    bytes.push(b'\n');
    std::fs::write(&path, bytes).with_context(|| format!("Cannot write {}", path.display()))?;
    Ok((path, count))
}
