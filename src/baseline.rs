//! The baseline of accepted findings: writing it from the last check, marking
//! why findings were accepted, with an optional note, listing the marks, and
//! counting their reasons per rule.
use crate::{
    options::Disposition,
    schema::{Report, Scope, Strength},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const BASELINE_FILE: &str = "jevgate-baseline.json";

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
    /// The function, type or other unit the finding names, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    strength: Option<Strength>,
    message: String,
    /// Why it was accepted, when someone said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<Disposition>,
    /// A short note with the reason, such as the issue a `later` finding
    /// will be fixed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

/// A note is one line of at most this many characters.
pub const NOTE_CHARS: usize = 200;

/// The note `baseline mark --note` records: one line, trimmed, of at most
/// [`NOTE_CHARS`] characters. An empty note clears the one a finding has.
pub fn note(text: &str) -> Result<Option<String>> {
    let text = text.trim();
    ensure!(
        !text.chars().any(char::is_control),
        "A note is one line of text, without line breaks or tabs"
    );
    ensure!(
        text.chars().count() <= NOTE_CHARS,
        "A note is at most {NOTE_CHARS} characters; link an issue for more"
    );
    Ok((!text.is_empty()).then(|| text.to_string()))
}

/// A baseline is read whole up to this size.
pub const BASELINE_BYTES: u64 = 16 * 1024 * 1024;

/// The committed baseline, when there is one.
fn read_baseline(root: &Path) -> Result<Option<Baseline>> {
    let path = root.join(BASELINE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let text = crate::inventory::read_source(&path, BASELINE_BYTES)
        .with_context(|| format!("Cannot read {BASELINE_FILE}"))?;
    parse(&text).map(Some)
}

/// The baseline in Git tree `tree`, such as an agent turn's start.
fn baseline_in(root: &Path, tree: &str) -> Result<Option<Baseline>> {
    let path = Path::new(BASELINE_FILE);
    let mut texts = crate::revision::blobs(root, tree, &[path], BASELINE_BYTES)?;
    texts.remove(path).as_deref().map(parse).transpose()
}

fn parse(text: &str) -> Result<Baseline> {
    let baseline: Baseline =
        serde_json::from_str(text).with_context(|| format!("Invalid {BASELINE_FILE}"))?;
    ensure!(baseline.version == 1, "Unsupported {BASELINE_FILE} version");
    Ok(baseline)
}

/// Whether `text` is a baseline JevGate can read.
pub(crate) fn parses(text: &str) -> bool {
    parse(text).is_ok()
}

/// Mark findings whose fingerprints the baseline accepted: the committed
/// one, or, within an agent's turn, the one in Git tree `as_of`, the turn's
/// start, and the entries the turn added that dismiss a finding with a
/// reason (see [`dismissed_since`]).
pub fn apply(root: &Path, report: &mut Report, as_of: Option<&str>) -> Result<()> {
    let accepted: BTreeSet<String> = match as_of {
        Some(tree) => {
            let then = baseline_in(root, tree)?.map_or_else(Vec::new, |b| b.findings);
            let dismissed = dismissed_since(root, tree).unwrap_or_default();
            then.into_iter()
                .map(|f| f.fingerprint)
                .chain(dismissed.into_keys())
                .collect()
        }
        None => read_baseline(root)?
            .map_or_else(Vec::new, |b| b.findings)
            .into_iter()
            .map(|f| f.fingerprint)
            .collect(),
    };
    for finding in report.files.iter_mut().flat_map(|f| &mut f.findings) {
        finding.baselined = accepted.contains(&finding.fingerprint);
    }
    Ok(())
}

/// Why a finding was dismissed: its reason, and the note given with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Dismissal {
    pub reason: Disposition,
    pub note: Option<String>,
}

/// The findings the baseline dismisses with a reason that its version in Git
/// tree `since` did not accept, by fingerprint: what an agent dismissed with
/// `baseline mark` during a turn that began at `since`. Accepting a finding
/// is otherwise the person's call, so an entry without a reason counts from
/// the next turn; a person audits the reasons with `baseline stats`.
pub fn dismissed_since(root: &Path, since: &str) -> Result<BTreeMap<String, Dismissal>> {
    let held: BTreeSet<String> = baseline_in(root, since)?
        .map_or_else(Vec::new, |b| b.findings)
        .into_iter()
        .map(|f| f.fingerprint)
        .collect();
    Ok(read_baseline(root)?
        .map_or_else(Vec::new, |b| b.findings)
        .into_iter()
        .filter(|f| !held.contains(&f.fingerprint))
        .filter_map(|f| {
            let reason = f.reason?;
            Some((
                f.fingerprint,
                Dismissal {
                    reason,
                    note: f.note,
                },
            ))
        })
        .collect())
}

/// What `jevgate baseline` wrote: the file, the findings it accepted from the
/// last check, and the earlier entries it kept for files that check did not cover.
pub struct Written {
    pub path: std::path::PathBuf,
    pub accepted: usize,
    pub kept: usize,
}

/// The last check, when its findings can make a baseline: it finished, and
/// it covered the repository unless `merge` keeps the files it did not.
fn last_check(root: &Path, merge: bool) -> Result<Report> {
    let report = crate::storage::read_latest(root)
        .context("No compatible .jevgate/latest.json; run jevgate check first")?;
    ensure!(
        report.complete && !report.dry_run,
        "The last check was incomplete; rerun it before writing a baseline"
    );
    // A coding agent's hook checks a few files at a time; replacing the
    // baseline with them would drop what was accepted for every other file.
    ensure!(
        merge || report.command != crate::hook::REPORT_COMMAND,
        "The last check was the agent hook's, of {}; run jevgate check first, or accept its findings with --merge, which keeps the rest",
        crate::output::count(report.files.len(), "file")
    );
    Ok(report)
}

/// Accept every finding of the last complete check. No source is read or sent.
/// With `merge`, earlier entries stay for files the check did not cover, such
/// as unchanged files of a `--base` run; entries for checked or deleted files
/// are replaced by what the check found. A check of changed lines judged only
/// what its change touched, so the entries of the files it checked stay too,
/// and only deleted files' entries go. A finding accepted before keeps its
/// reason and note; the others get `reason`.
pub fn write(root: &Path, merge: bool, reason: Option<Disposition>) -> Result<Written> {
    let report = last_check(root, merge)?;
    let mut findings = to_accept(&report, reason);
    let accepted = findings.len();
    let previous = read_baseline(root)?;
    if let Some(previous) = &previous {
        keep_marks(&mut findings, previous);
    }
    let earlier = match previous {
        Some(previous) if merge => uncovered(&report, previous),
        _ => Vec::new(),
    };
    let kept = earlier.len();
    findings.extend(earlier);
    in_file_order(&mut findings);
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

/// The check's findings to accept, each with `reason`. A suppressed finding
/// is accepted where its comment is; removing the comment brings it back.
fn to_accept(report: &Report, reason: Option<Disposition>) -> Vec<Accepted> {
    report
        .files
        .iter()
        .flat_map(|file| {
            file.findings
                .iter()
                .filter(|f| f.suppressed.is_none())
                .map(|f| Accepted {
                    fingerprint: f.fingerprint.clone(),
                    rule: f.rule.clone(),
                    path: file.path.clone(),
                    line: Some(f.line),
                    unit: f.symbol.clone(),
                    strength: Some(f.strength),
                    message: f.message.clone(),
                    reason,
                    note: None,
                })
        })
        .collect()
}

/// Give each finding accepted before the reason it was accepted with, and
/// its note.
fn keep_marks(findings: &mut [Accepted], previous: &Baseline) {
    let marks: BTreeMap<&str, &Accepted> = previous
        .findings
        .iter()
        .filter(|f| f.reason.is_some() || f.note.is_some())
        .map(|f| (f.fingerprint.as_str(), f))
        .collect();
    for finding in findings {
        if let Some(earlier) = marks.get(finding.fingerprint.as_str()) {
            finding.reason = earlier.reason.or(finding.reason);
            finding.note.clone_from(&earlier.note);
        }
    }
}

/// The earlier entries a merge keeps: those of files the check did not
/// cover. A check of changed lines covers only the files it deleted.
fn uncovered(report: &Report, previous: Baseline) -> Vec<Accepted> {
    let whole = report.scope == Scope::WholeFiles;
    let covered: BTreeSet<&Path> = report
        .files
        .iter()
        .filter(|_| whole)
        .map(|f| f.path.as_path())
        .chain(report.deleted_files.iter().map(|p| p.as_path()))
        .collect();
    previous
        .findings
        .into_iter()
        .filter(|f| !covered.contains(f.path.as_path()))
        .collect()
}

fn save_baseline(root: &Path, baseline: &Baseline) -> Result<std::path::PathBuf> {
    let path = root.join(BASELINE_FILE);
    let mut bytes = serde_json::to_vec_pretty(baseline)?;
    bytes.push(b'\n');
    std::fs::write(&path, bytes).with_context(|| format!("Cannot write {}", path.display()))?;
    Ok(path)
}

/// What `baseline mark` records, and on which findings.
pub struct Mark<'a> {
    pub reason: Disposition,
    /// The note to record, already checked by [`note`]: `None` keeps the
    /// note a finding has, `Some(None)` clears it.
    pub note: Option<Option<String>>,
    /// Paths or directories, `PATH:LINE`s or fingerprint prefixes.
    pub targets: &'a [String],
    /// Rule keys; every rule when empty.
    pub rules: &'a [&'a str],
}

/// Record a reason, and a note when given, on the findings a target names
/// whose rule is among the mark's rules; returns how many were marked. A
/// target is a path or directory, `PATH:LINE`, or a fingerprint prefix of
/// at least 8 characters. Accepted findings are marked; a finding of the
/// last check that the baseline does not hold yet is accepted with the
/// reason when a target names it by `PATH:LINE` or fingerprint: a coding
/// agent dismisses what its hook reported this way. A path or directory
/// accepts nothing new, so no finding is dismissed unread.
pub fn mark(root: &Path, mark: &Mark<'_>) -> Result<usize> {
    let targets = mark.targets;
    // An empty path would be the start of every path.
    ensure!(
        targets
            .iter()
            .all(|t| !t.trim().trim_end_matches('/').is_empty()),
        "An empty target names no finding; give PATH:LINE, a fingerprint or a path"
    );
    let mut baseline = read_baseline(root)?.unwrap_or_else(|| Baseline {
        version: 1,
        created_at: crate::schema::now(),
        findings: Vec::new(),
    });
    let named = |finding: &Accepted, paths: bool| {
        selected(&finding.rule, mark.rules) && targets.iter().any(|t| names(t, finding, paths))
    };
    let mut marked = 0;
    for finding in &mut baseline.findings {
        if named(finding, true) {
            finding.reason = Some(mark.reason);
            if let Some(note) = &mark.note {
                finding.note.clone_from(note);
            }
            marked += 1;
        }
    }
    if let Ok(report) = crate::storage::read_latest(root) {
        let note = mark.note.clone().flatten();
        marked += add_dismissed(&mut baseline, &report, (mark.reason, note), |f| {
            named(f, false)
        });
    }
    ensure!(
        marked > 0,
        "No finding of the last check or {BASELINE_FILE} matches {}",
        targets.join(", ")
    );
    save_baseline(root, &baseline)?;
    Ok(marked)
}

/// Whether a finding of `rule` is among `rules`, rule keys: every rule when
/// there are none.
fn selected(rule: &str, rules: &[&str]) -> bool {
    let key = match crate::catalog::find(rule) {
        Some(found) => Some(found.key),
        None => crate::catalog::custom(rule).then_some(rule),
    };
    rules.is_empty() || key.is_some_and(|key| rules.contains(&key))
}

/// Adds the findings of `report` that `named` picks and `baseline` does not
/// hold yet, dismissed for the reason with the note, and says how many it
/// added.
fn add_dismissed(
    baseline: &mut Baseline,
    report: &Report,
    (reason, note): (Disposition, Option<String>),
    named: impl Fn(&Accepted) -> bool,
) -> usize {
    let held: BTreeSet<String> = baseline
        .findings
        .iter()
        .map(|f| f.fingerprint.clone())
        .collect();
    let mut dismissed: Vec<Accepted> = to_accept(report, Some(reason))
        .into_iter()
        .filter(|f| !held.contains(&f.fingerprint) && named(f))
        .map(|f| Accepted {
            note: note.clone(),
            ..f
        })
        .collect();
    in_file_order(&mut dismissed);
    let added = dismissed.len();
    baseline.findings.extend(dismissed);
    in_file_order(&mut baseline.findings);
    added
}

/// The order the baseline file keeps its entries in, by path, then
/// fingerprint, each finding once.
fn in_file_order(findings: &mut Vec<Accepted>) {
    findings.sort_by(|a, b| (&a.path, &a.fingerprint).cmp(&(&b.path, &b.fingerprint)));
    // A fingerprint covers the path, so a finding's copies are adjacent.
    findings.dedup_by(|a, b| a.fingerprint == b.fingerprint);
}

/// Whether a `mark` target names a finding: by fingerprint prefix or
/// `PATH:LINE`, or with `paths`, by a path or directory holding it.
fn names(target: &str, finding: &Accepted, paths: bool) -> bool {
    const FINGERPRINT_PREFIX: usize = 8;
    if target.len() >= FINGERPRINT_PREFIX && finding.fingerprint.starts_with(target) {
        return true;
    }
    if let Some((path, line)) = target.rsplit_once(':')
        && let Ok(line) = line.parse::<usize>()
    {
        return finding.path == Path::new(path) && finding.line == Some(line);
    }
    let target = target.trim_end_matches('/');
    paths && !target.is_empty() && finding.path.starts_with(target)
}

/// An accepted finding as `baseline list` prints it.
#[derive(Debug, Serialize, PartialEq)]
pub struct Listed {
    pub path: std::path::PathBuf,
    pub line: Option<usize>,
    pub reason: Option<Disposition>,
    pub rule: String,
    pub unit: Option<String>,
    pub fingerprint: String,
    pub note: Option<String>,
    pub message: String,
}

/// The fingerprint prefix the listing prints: as many characters as
/// `baseline mark` needs to name a finding.
const LISTED_PREFIX: usize = 8;

/// The accepted findings with one of `reasons` (any, or none, when empty)
/// whose rule is among `rules` (every rule when empty), by path, then line.
pub fn list(root: &Path, reasons: &[Disposition], rules: &[&str]) -> Result<Vec<Listed>> {
    let baseline = read_baseline(root)?
        .with_context(|| format!("No {BASELINE_FILE}; run jevgate baseline first"))?;
    let mut listed: Vec<Listed> = baseline
        .findings
        .into_iter()
        .filter(|f| reasons.is_empty() || f.reason.is_some_and(|r| reasons.contains(&r)))
        .filter(|f| selected(&f.rule, rules))
        .map(|f| Listed {
            path: f.path,
            line: f.line,
            reason: f.reason,
            rule: f.rule,
            unit: f.unit,
            fingerprint: f.fingerprint,
            note: f.note,
            message: f.message,
        })
        .collect();
    listed
        .sort_by(|a, b| (&a.path, a.line, &a.fingerprint).cmp(&(&b.path, b.line, &b.fingerprint)));
    Ok(listed)
}

impl Listed {
    fn location(&self) -> String {
        match self.line {
            Some(line) => format!("{}:{line}", self.path.display()),
            None => self.path.display().to_string(),
        }
    }

    fn reason(&self) -> String {
        self.reason
            .map_or_else(|| "no reason".into(), |r| crate::output::label(&r))
    }

    fn id(&self) -> &str {
        self.fingerprint
            .get(..LISTED_PREFIX)
            .unwrap_or(&self.fingerprint)
    }
}

/// The listing for people: location, reason, rule and fingerprint prefix
/// in aligned columns, then the unit and the note.
pub fn list_text(listed: &[Listed]) -> String {
    let rows: Vec<[String; 4]> = listed
        .iter()
        .map(|l| [l.location(), l.reason(), l.rule.clone(), l.id().to_string()])
        .collect();
    let widths: Vec<usize> = (0..4)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .zip(listed)
        .map(|(row, l)| {
            let mut cells: Vec<String> = row
                .iter()
                .zip(&widths)
                .map(|(cell, &width)| format!("{cell:<width$}"))
                .collect();
            cells.extend(l.unit.clone());
            cells.extend(l.note.as_ref().map(|note| format!("note: {note}")));
            cells.join("  ").trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The listing as a Markdown checklist to paste into a cleanup issue.
pub fn list_markdown(listed: &[Listed]) -> String {
    // A unit named with its own code spans stays as it is.
    let code = |text: &str| {
        if text.contains('`') {
            text.to_string()
        } else {
            format!("`{text}`")
        }
    };
    listed
        .iter()
        .map(|l| {
            let mut line = format!("- [ ] {} {}", code(&l.location()), l.rule);
            if let Some(unit) = &l.unit {
                line.push_str(&format!(" {}", code(unit)));
            }
            line.push_str(&format!(" ({}, {})", l.reason(), code(l.id())));
            if let Some(note) = &l.note {
                line.push_str(&format!(": {note}"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
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
