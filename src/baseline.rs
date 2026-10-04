//! The baseline of accepted findings: writing it from the last check, marking
//! why findings were accepted, with an optional note, listing the marks, and
//! counting their reasons per rule. Every write rewrites the entries a
//! finding of the last check matched through a fingerprint it had before
//! (`entries`), so a baseline moves to the current fingerprints keeping
//! each reason and note.
use crate::{
    options::Disposition,
    schema::{Finding, Report, Scope, Strength},
};
use anyhow::{Context, Result, ensure};
use entries::{Entries, Found};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

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
    /// What a finding whose fingerprint changed is matched by: a repeat's
    /// copies, an outline's member names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    members: Vec<String>,
}

mod entries;
mod listing;

pub use listing::{list, list_markdown, list_text, stats, stats_table};

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

/// The entries of a baseline read: none without one.
fn entries(baseline: Result<Option<Baseline>>) -> Result<Vec<Accepted>> {
    Ok(baseline?.map_or_else(Vec::new, |b| b.findings))
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

/// Mark the findings the baseline accepts (`Entries::find`): the
/// committed one, or, within an agent's turn, the one in Git tree `as_of`,
/// the turn's start, and the entries the turn added that dismiss a finding
/// with a reason (see [`Dismissals`]).
pub fn apply(root: &Path, report: &mut Report, as_of: Option<&str>) -> Result<()> {
    let accepts: Box<dyn Fn(&Finding) -> bool> = match as_of {
        Some(tree) => {
            let dismissals = Dismissals::since(root, tree)?;
            Box::new(move |finding| {
                dismissals.then.find(finding).is_some() || dismissals.of(finding).is_some()
            })
        }
        None => {
            let entries = Entries::new(entries(read_baseline(root))?);
            Box::new(move |finding| entries.find(finding).is_some())
        }
    };
    for finding in report.files.iter_mut().flat_map(|f| &mut f.findings) {
        finding.baselined = accepts(finding);
    }
    Ok(())
}

/// Why a finding was dismissed: its reason, and the note given with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Dismissal {
    pub reason: Disposition,
    pub note: Option<String>,
}

/// The baseline as an agent's turn began and as it is now, which tell the
/// findings the agent dismissed with `baseline mark` during the turn.
pub struct Dismissals {
    then: Entries,
    now: Entries,
}

impl Dismissals {
    /// The baseline in Git tree `since`, the turn's start, and the
    /// committed one; a baseline the turn left unreadable dismisses nothing.
    pub fn since(root: &Path, since: &str) -> Result<Self> {
        let then = entries(baseline_in(root, since))?;
        let now = entries(read_baseline(root)).unwrap_or_default();
        Ok(Self {
            then: Entries::new(then),
            now: Entries::new(now),
        })
    }

    /// The dismissal of `finding` with a reason that the baseline as the
    /// turn began did not accept. Accepting a finding is otherwise the
    /// person's call, so an entry without a reason counts from the next
    /// turn; a person audits the reasons with `baseline stats`. An entry a
    /// write rewrote to the finding's current fingerprint was accepted
    /// before, through the fingerprint it had then.
    pub fn of(&self, finding: &Finding) -> Option<Dismissal> {
        if self.then.find(finding).is_some() {
            return None;
        }
        let entry = self.now.get(self.now.find(finding)?);
        Some(Dismissal {
            reason: entry.reason?,
            note: entry.note.clone(),
        })
    }
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
/// reason and note, at its current fingerprint; the others get `reason`.
pub fn write(root: &Path, merge: bool, reason: Option<Disposition>) -> Result<Written> {
    let report = last_check(root, merge)?;
    let previous = Entries::new(entries(read_baseline(root))?);
    let mut findings = Vec::new();
    // Entries a finding now stands for, which a merge does not keep.
    let mut replaced = BTreeSet::new();
    for (path, finding) in accepted_findings(&report) {
        let entry = accepted(path, finding, reason);
        findings.push(match previous.find(finding) {
            Some(found) => {
                if !matches!(found, Found::Copies(_)) {
                    replaced.insert(found.at());
                }
                kept(entry, previous.get(found))
            }
            None => entry,
        });
    }
    let accepted = findings.len();
    let earlier = if merge {
        let kept = previous
            .entries
            .into_iter()
            .enumerate()
            .filter(|(at, _)| !replaced.contains(at))
            .map(|(_, entry)| entry);
        uncovered(&report, kept)
    } else {
        Vec::new()
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

/// The check's findings to accept, with their files' paths. A suppressed
/// finding is accepted where its comment is; removing the comment brings
/// it back.
fn accepted_findings(report: &Report) -> impl Iterator<Item = (&Path, &Finding)> {
    report.files.iter().flat_map(|file| {
        file.findings
            .iter()
            .filter(|f| f.suppressed.is_none())
            .map(|f| (file.path.as_path(), f))
    })
}

/// The entry that accepts `finding`, in the file at `path`, with `reason`.
fn accepted(path: &Path, finding: &Finding, reason: Option<Disposition>) -> Accepted {
    Accepted {
        fingerprint: finding.fingerprint.clone(),
        rule: finding.rule.clone(),
        path: path.to_path_buf(),
        line: Some(finding.line),
        unit: finding.symbol.clone(),
        strength: Some(finding.strength),
        message: finding.message.clone(),
        reason,
        note: None,
        members: finding.identity.members.clone(),
    }
}

/// `entry` with the reason and note of the `earlier` one it replaces; its
/// own reason when that one had none.
fn kept(mut entry: Accepted, earlier: &Accepted) -> Accepted {
    entry.reason = earlier.reason.or(entry.reason);
    entry.note.clone_from(&earlier.note);
    entry
}

/// The earlier entries a merge keeps: those of files the check did not
/// cover. A check of changed lines covers only the files it deleted.
fn uncovered(report: &Report, previous: impl Iterator<Item = Accepted>) -> Vec<Accepted> {
    let whole = report.scope == Scope::WholeFiles;
    let covered: BTreeSet<&Path> = report
        .files
        .iter()
        .filter(|_| whole)
        .map(|f| f.path.as_path())
        .chain(report.deleted_files.iter().map(|p| p.as_path()))
        .collect();
    previous
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
    let report = crate::storage::read_latest(root).ok();
    if let Some(report) = &report {
        rewrite(&mut baseline, report);
    }
    let mut marked = mark_entries(&mut baseline.findings, mark, |f| named(f, true));
    if let Some(report) = &report {
        let note = mark.note.clone().flatten();
        marked += add_dismissed(&mut baseline, report, (mark.reason, note), |f| {
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

/// Record the mark's reason, and its note when given, on the entries
/// `named` picks; returns how many.
fn mark_entries(
    findings: &mut [Accepted],
    mark: &Mark<'_>,
    named: impl Fn(&Accepted) -> bool,
) -> usize {
    let mut marked = 0;
    for finding in findings.iter_mut().filter(|f| named(f)) {
        finding.reason = Some(mark.reason);
        if let Some(note) = &mark.note {
            finding.note.clone_from(note);
        }
        marked += 1;
    }
    marked
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

/// Rewrite each entry a finding of `report` matched through a fingerprint
/// it had before to the finding's current fingerprint, keeping the entry's
/// reason and note.
fn rewrite(baseline: &mut Baseline, report: &Report) {
    let entries = Entries::new(std::mem::take(&mut baseline.findings));
    let mut replaced = BTreeSet::new();
    let mut rewritten = Vec::new();
    for (path, finding) in accepted_findings(report) {
        if let Some(Found::Earlier(at)) = entries.find(finding) {
            rewritten.push(kept(accepted(path, finding, None), &entries.entries[at]));
            replaced.insert(at);
        }
    }
    baseline.findings = entries
        .entries
        .into_iter()
        .enumerate()
        .filter(|(at, _)| !replaced.contains(at))
        .map(|(_, entry)| entry)
        .chain(rewritten)
        .collect();
    in_file_order(&mut baseline.findings);
}

/// Adds the findings of `report` that `named` picks and `baseline` does not
/// accept yet, dismissed for the reason with the note, and says how many it
/// added. One replaces an entry with its fingerprint that no longer
/// accepts it, as an outline's whose members changed too much.
fn add_dismissed(
    baseline: &mut Baseline,
    report: &Report,
    (reason, note): (Disposition, Option<String>),
    named: impl Fn(&Accepted) -> bool,
) -> usize {
    let held = Entries::new(std::mem::take(&mut baseline.findings));
    let mut dismissed: Vec<Accepted> = accepted_findings(report)
        .filter(|(_, finding)| held.find(finding).is_none())
        .map(|(path, finding)| Accepted {
            note: note.clone(),
            ..accepted(path, finding, Some(reason))
        })
        .filter(|f| named(f))
        .collect();
    in_file_order(&mut dismissed);
    let added = dismissed.len();
    let replaced: BTreeSet<&str> = dismissed.iter().map(|f| f.fingerprint.as_str()).collect();
    let mut findings: Vec<Accepted> = held
        .entries
        .into_iter()
        .filter(|f| !replaced.contains(f.fingerprint.as_str()))
        .collect();
    findings.extend(dismissed);
    in_file_order(&mut findings);
    baseline.findings = findings;
    added
}

/// The order the baseline file keeps its entries in, by path, then
/// fingerprint, each finding once: the first entry with its fingerprint.
fn in_file_order(findings: &mut Vec<Accepted>) {
    let mut seen = BTreeSet::new();
    findings.retain(|f| seen.insert(f.fingerprint.clone()));
    findings.sort_by(|a, b| (&a.path, &a.fingerprint).cmp(&(&b.path, &b.fingerprint)));
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
