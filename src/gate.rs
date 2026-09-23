//! The configurable quality gate: which results fail a check, and a baseline
//! of accepted findings. Classification never depends on this policy.
use crate::{
    options::{CheckArgs, Disposition, FailOn},
    schema::{Finding, Report, Status, Strength},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    strength: Option<Strength>,
    message: String,
    /// Why it was accepted, when someone said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<Disposition>,
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
    let new: Vec<_> = findings.filter(|(_, f)| !f.baselined).collect();
    let reasons = failures(report, &new, args);
    report.gate = report.complete.then_some(Gate {
        passed: reasons.is_empty(),
        reasons,
        new_findings: new.len(),
        baselined_findings: baselined,
    });
}

/// Whether a finding in `path` fails the gate: new, not a note, and at its
/// rule's level for that path. Consider is the lower bar, so it also fails
/// on review findings.
pub fn fails(finding: &Finding, path: &Path, args: &CheckArgs) -> bool {
    let levels = args.levels_at(&finding.rule, path);
    !finding.baselined
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
/// are replaced by what the check found. A finding accepted before keeps its
/// reason; the others get `reason`.
pub fn write_baseline(root: &Path, merge: bool, reason: Option<Disposition>) -> Result<Written> {
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
                line: Some(f.line),
                strength: Some(f.strength),
                message: f.message.clone(),
                reason,
            })
        })
        .collect();
    let accepted = findings.len();
    let mut kept = 0;
    let previous = read_baseline(root)?;
    if let Some(previous) = &previous {
        let reasons: BTreeMap<&str, Disposition> = previous
            .findings
            .iter()
            .filter_map(|f| Some((f.fingerprint.as_str(), f.reason?)))
            .collect();
        for finding in &mut findings {
            if let Some(earlier) = reasons.get(finding.fingerprint.as_str()) {
                finding.reason = Some(*earlier);
            }
        }
    }
    if merge && let Some(previous) = previous {
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
    let path = save_baseline(
        root,
        &Baseline {
            version: 1,
            created_at: crate::schema::now(),
            findings,
        },
    )?;
    Ok(Written {
        path,
        accepted,
        kept,
    })
}

fn save_baseline(root: &Path, baseline: &Baseline) -> Result<std::path::PathBuf> {
    let path = root.join(BASELINE_FILE);
    let mut bytes = serde_json::to_vec_pretty(baseline)?;
    bytes.push(b'\n');
    std::fs::write(&path, bytes).with_context(|| format!("Cannot write {}", path.display()))?;
    Ok(path)
}

/// Record `reason` on the accepted findings a target names and whose rule is
/// among `rules` (every rule when empty); returns how many were marked.
/// A target is a path or directory, `PATH:LINE`, or a fingerprint prefix of
/// at least 8 characters.
pub fn mark(root: &Path, reason: Disposition, targets: &[String], rules: &[&str]) -> Result<usize> {
    let mut baseline = read_baseline(root)?
        .with_context(|| format!("No {BASELINE_FILE}; run jevgate baseline first"))?;
    let mut marked = 0;
    for finding in &mut baseline.findings {
        let rule = crate::catalog::find(&finding.rule).map(|r| r.key);
        if (rules.is_empty() || rule.is_some_and(|key| rules.contains(&key)))
            && targets.iter().any(|t| names(t, finding))
        {
            finding.reason = Some(reason);
            marked += 1;
        }
    }
    ensure!(
        marked > 0,
        "No accepted finding matches {}",
        targets.join(", ")
    );
    save_baseline(root, &baseline)?;
    Ok(marked)
}

/// Whether a `mark` target names an accepted finding.
fn names(target: &str, finding: &Accepted) -> bool {
    const FINGERPRINT_PREFIX: usize = 8;
    if target.len() >= FINGERPRINT_PREFIX && finding.fingerprint.starts_with(target) {
        return true;
    }
    if let Some((path, line)) = target.rsplit_once(':')
        && let Ok(line) = line.parse::<usize>()
    {
        return finding.path == Path::new(path) && finding.line == Some(line);
    }
    let target = Path::new(target.trim_end_matches('/'));
    finding.path.starts_with(target)
}

/// Accepted findings of one rule by reason.
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct ReasonCounts {
    pub accepted: usize,
    pub intended: usize,
    pub later: usize,
    pub wrong: usize,
    pub without_reason: usize,
    /// `wrong` among the findings with a reason; none when no finding has one.
    pub wrong_rate: Option<f64>,
}

/// Accepted findings by rule ID and reason.
pub fn stats(root: &Path) -> Result<BTreeMap<String, ReasonCounts>> {
    let baseline = read_baseline(root)?
        .with_context(|| format!("No {BASELINE_FILE}; run jevgate baseline first"))?;
    let mut counts = BTreeMap::<String, ReasonCounts>::new();
    for finding in &baseline.findings {
        let count = counts.entry(finding.rule.clone()).or_default();
        count.accepted += 1;
        *match finding.reason {
            Some(Disposition::Intended) => &mut count.intended,
            Some(Disposition::Later) => &mut count.later,
            Some(Disposition::Wrong) => &mut count.wrong,
            None => &mut count.without_reason,
        } += 1;
    }
    for count in counts.values_mut() {
        let reasoned = count.accepted - count.without_reason;
        count.wrong_rate = (reasoned > 0).then(|| count.wrong as f64 / reasoned as f64);
    }
    Ok(counts)
}

/// The stats as a table for people.
pub fn stats_table(counts: &BTreeMap<String, ReasonCounts>) -> String {
    let width = counts.keys().map(String::len).max().unwrap_or(0).max(4);
    let mut lines = vec![format!(
        "{:<width$} {:>8} {:>8} {:>6} {:>6} {:>9} {:>6}",
        "rule", "accepted", "intended", "later", "wrong", "no reason", "wrong%"
    )];
    for (rule, c) in counts {
        let rate = c
            .wrong_rate
            .map_or("-".to_string(), |r| format!("{:.0}%", r * 100.0));
        lines.push(format!(
            "{rule:<width$} {:>8} {:>8} {:>6} {:>6} {:>9} {rate:>6}",
            c.accepted, c.intended, c.later, c.wrong, c.without_reason
        ));
    }
    lines.join("\n")
}
