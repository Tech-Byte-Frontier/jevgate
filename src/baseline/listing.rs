//! The baseline's entries as `baseline list` and `baseline stats` print
//! them: by path with their reasons and notes, and counted by rule and reason.
use super::{BASELINE_FILE, read_baseline, selected};
use crate::options::Disposition;
use anyhow::{Context, Result};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

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
