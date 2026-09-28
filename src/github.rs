//! `--format github`: GitHub Actions workflow commands that annotate the
//! changed lines, a Markdown job summary, then the agent text for the log.
use crate::{
    options::CheckArgs,
    output,
    schema::{Finding, Report, Status, Strength},
};
use anyhow::Result;
use std::{io::Write, path::Path};

/// Summary rows before the rest are left to the JSON report.
const SUMMARY_ROWS: usize = 50;

/// Annotations for run errors, failed files and every new finding that is not
/// a note (an error when it fails the gate, else a warning, which says so when
/// its rule and level are still being measured), then the agent text. The
/// summary goes to `$GITHUB_STEP_SUMMARY` when the runner sets it.
pub fn emit(out: &mut impl Write, report: &Report, args: &CheckArgs) -> Result<()> {
    for error in &report.errors {
        writeln!(out, "::error title=JevGate run incomplete::{}", data(error))?;
    }
    for file in report.files.iter().filter(|f| f.status == Status::Error) {
        let error = file.error.as_deref().unwrap_or("Not judged");
        writeln!(
            out,
            "::error file={},title=JevGate could not judge this file::{}",
            property(&file.path.to_string_lossy()),
            data(error)
        )?;
    }
    let shown: Vec<(&Path, &Finding)> = output::failing_first(report)
        .into_iter()
        .filter(|(_, f)| f.strength != Strength::Note && !f.accepted())
        .collect();
    for (path, finding) in &shown {
        writeln!(out, "{}", annotation(path, finding))?;
    }
    if let Some(file) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let written = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&file)
            .and_then(|mut f| f.write_all(summary(report, &shown).as_bytes()));
        if let Err(error) = written {
            note!("jevgate: cannot write the job summary: {error}");
        }
    }
    output::agent(out, report, args.verbose, output::Style::PLAIN)
}

fn annotation(path: &Path, finding: &Finding) -> String {
    let end = finding
        .locations
        .iter()
        .find(|l| l.path == path && l.start_line == finding.line)
        .map_or(String::new(), |l| format!(",endLine={}", l.end_line));
    let mut message = format!("{}\n→ {}", finding.message, finding.action);
    if let Some(note) = output::measuring_note(finding) {
        message.push_str(&format!("\n{note}"));
    }
    format!(
        "::{} file={},line={}{end},title={}::{}",
        if finding.fails_gate() {
            "error"
        } else {
            "warning"
        },
        property(&path.to_string_lossy()),
        finding.line,
        property(&format!("JevGate {} [{}]", label(finding), finding.rule)),
        data(&message),
    )
}

fn label(finding: &Finding) -> String {
    output::label(&finding.strength)
}

/// The Markdown job summary: the headline, then a table of findings, with
/// the ones that fail the gate in bold.
fn summary(report: &Report, shown: &[(&Path, &Finding)]) -> String {
    let mut text = format!("### {}\n\n", output::headline(report));
    for error in &report.errors {
        text.push_str(&format!("- **Error:** {}\n", cell(error)));
    }
    let failed = report
        .files
        .iter()
        .filter(|f| f.status == Status::Error)
        .count();
    if failed > 0 {
        text.push_str(&format!(
            "- **{} not judged;** the annotations give each reason.\n\n",
            crate::output::count(failed, "file")
        ));
    }
    if shown.is_empty() {
        text.push_str("No new review or consider findings.\n\n");
        return text;
    }
    text.push_str("| | Location | Rule | Finding |\n|---|---|---|---|\n");
    for (path, finding) in shown.iter().take(SUMMARY_ROWS) {
        let level = if finding.fails_gate() {
            format!("**{}**", label(finding))
        } else {
            label(finding)
        };
        text.push_str(&format!(
            "| {level} | `{}:{}` | `{}` | {} → {} |\n",
            cell(&path.to_string_lossy()),
            finding.line,
            finding.rule,
            cell(&finding.message),
            cell(&finding.action)
        ));
    }
    if shown.len() > SUMMARY_ROWS {
        text.push_str(&format!(
            "\n{} more in `.jevgate/latest.json`.\n",
            shown.len() - SUMMARY_ROWS
        ));
    }
    if let Some(line) = output::measuring(report) {
        text.push_str(&format!("\n{line}\n"));
    }
    text.push('\n');
    text
}

/// A workflow command message: `%`, CR and LF are escaped.
fn data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// A workflow command property value: also `:` and `,`.
fn property(text: &str) -> String {
    data(text).replace(':', "%3A").replace(',', "%2C")
}

/// A Markdown table cell on one line.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{schema::Gating, tests::finding};

    fn counted(strength: Strength, gate: Gating) -> Finding {
        Finding {
            gate: Some(gate),
            ..finding(strength)
        }
    }

    #[test]
    fn annotations_escape_commands_and_mark_what_fails_the_gate() {
        let review = counted(Strength::Review, Gating::Fails);
        let line = annotation(Path::new("src/a,b.rs"), &review);
        assert_eq!(
            line,
            "::error file=src/a%2Cb.rs,line=12,endLine=20,title=JevGate review [maintainability/shared-logic]::Copies: 50%25 alike,%0Asee `b`%0A→ Share one | implementation"
        );
        assert!(!line.contains('\n'));
        let consider = annotation(Path::new("x.rs"), &finding(Strength::Consider));
        assert!(consider.starts_with("::warning file=x.rs,line=12,title="));
    }

    #[test]
    fn a_review_still_being_measured_is_a_warning_that_says_why() {
        let review = counted(Strength::Review, Gating::Measuring);
        let line = annotation(Path::new("x.rs"), &review);
        assert!(line.starts_with("::warning file=x.rs,"), "{line}");
        assert!(
            line.ends_with("%0ADoes not fail the gate: maintainability/shared-logic reviews are still being measured (54%25 of 85 right on projects JevGate was never tuned on)."),
            "{line}"
        );
    }

    #[test]
    fn the_summary_says_why_reviews_still_being_measured_did_not_fail() {
        let report = crate::tests::gated(
            vec![crate::tests::finding_of(
                "maintainability/shared-logic",
                Strength::Review,
            )],
            &crate::tests::args(),
        );
        let shown = output::failing_first(&report);
        let text = summary(&report, &shown);
        assert!(text.contains("| review | `src/lib.rs:12`"), "{text}");
        assert!(
            text.contains("\n1 review did not fail the gate: by default only rules and levels"),
            "{text}"
        );
    }

    #[test]
    fn summary_rows_stay_on_one_line_and_bold_gate_failures() {
        let args = crate::tests::args();
        let review = counted(Strength::Review, Gating::Fails);
        let consider = counted(Strength::Consider, Gating::Measuring);
        let path = Path::new("src/a.rs");
        let report = crate::evaluate::snapshot(
            &[],
            &Default::default(),
            &args,
            crate::evaluate::SnapshotContext {
                root: Path::new("."),
                generation: 1,
                requests: 0,
            },
        );
        let text = summary(&report, &[(path, &review), (path, &consider)]);
        let rows: Vec<&str> = text.lines().filter(|l| l.starts_with("| ")).collect();
        assert_eq!(rows.len(), 3, "{text}");
        assert!(
            rows[1].starts_with("| **review** | `src/a.rs:12`"),
            "{text}"
        );
        assert!(rows[1].contains("50% alike, see `b` → Share one \\| implementation"));
        assert!(rows[2].starts_with("| consider |"));
    }
}
