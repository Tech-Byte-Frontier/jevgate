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
    // Notices, which do not take the error and warning slots findings use.
    for guard in &report.guards {
        let line = guard.line.map_or(String::new(), |l| format!(",line={l}"));
        writeln!(
            out,
            "::notice file={}{line},title=JevGate guard::{}",
            property(&guard.path.to_string_lossy()),
            data(&guard.describe())
        )?;
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

/// The Markdown job summary: the headline, a table of findings, with the
/// ones that fail the gate in bold, then the guards.
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
    } else {
        text.push_str(&findings_table(shown));
        if let Some(line) = output::measuring(report) {
            text.push_str(&format!("{line}\n\n"));
        }
    }
    text.push_str(&guards_list(&report.guards));
    text
}

/// The guards as a Markdown list; nothing without any.
fn guards_list(guards: &[crate::guards::Guard]) -> String {
    if guards.is_empty() {
        return String::new();
    }
    let heading = format!(
        "**Guards ({}):** {}.\n\n",
        guards.len(),
        output::GUARDS_HEADING
    );
    heading + &capped(guards.iter().map(|g| format!("- {}", cell(&g.describe()))))
}

/// The findings as a Markdown table, those that fail the gate in bold.
fn findings_table(shown: &[(&Path, &Finding)]) -> String {
    let rows = shown.iter().map(|(path, finding)| {
        let level = if finding.fails_gate() {
            format!("**{}**", label(finding))
        } else {
            label(finding)
        };
        format!(
            "| {level} | `{}:{}` | `{}` | {} → {} |",
            cell(&path.to_string_lossy()),
            finding.line,
            finding.rule,
            cell(&finding.message),
            cell(&finding.action)
        )
    });
    String::from("| | Location | Rule | Finding |\n|---|---|---|---|\n") + &capped(rows)
}

/// The first [`SUMMARY_ROWS`] of `rows`, a line each, then how many more
/// the JSON report holds.
fn capped(rows: impl ExactSizeIterator<Item = String>) -> String {
    let more = rows.len().saturating_sub(SUMMARY_ROWS);
    let mut text: String = rows.take(SUMMARY_ROWS).map(|row| row + "\n").collect();
    if more > 0 {
        text.push_str(&format!("\n{more} more in `.jevgate/latest.json`.\n"));
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
    fn a_review_that_does_not_fail_is_annotated_before_higher_ranked_considers() {
        use crate::tests::finding_of;
        let ranked = |rule: &str, strength: Strength, rank: f64| Finding {
            rank,
            ..finding_of(rule, strength)
        };
        let considers =
            (0..11).map(|_| ranked("maintainability/shared-logic", Strength::Consider, 0.9));
        let measured = ranked("maintainability/shared-logic", Strength::Review, 0.5);
        let failing = ranked(
            "maintainability/function-simplification",
            Strength::Review,
            0.1,
        );
        let report = crate::tests::gated(
            considers.chain([measured, failing]).collect(),
            &crate::tests::args(),
        );
        let order: Vec<(Strength, bool)> = output::failing_first(&report)
            .iter()
            .map(|(_, f)| (f.strength, f.fails_gate()))
            .collect();
        assert_eq!(
            order[..3],
            [
                (Strength::Review, true),
                (Strength::Review, false),
                (Strength::Consider, false)
            ],
            "the review still being measured is the first warning, not the twelfth"
        );
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

    #[test]
    fn guards_are_notices_and_a_list_in_the_summary() {
        let guards: Vec<crate::guards::Guard> = (1..=52)
            .map(|line| {
                serde_json::from_value(serde_json::json!({
                    "kind": "suppression", "path": "a,b.py", "line": line, "text": "x = 1 # noqa: E501",
                    "message": "turns off flake8 or Ruff here", "id": line.to_string()
                }))
                .unwrap()
            })
            .collect();
        let mut report = crate::evaluate::snapshot(
            &[],
            &Default::default(),
            &crate::tests::args(),
            crate::evaluate::SnapshotContext {
                root: Path::new("."),
                generation: 1,
                requests: 0,
            },
        );
        report.guards = guards;
        let mut out = Vec::new();
        emit(&mut out, &report, &crate::tests::args()).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.starts_with("::notice file=a%2Cb.py,line=1,title=JevGate guard::a,b.py:1 turns off flake8 or Ruff here: x = 1 # noqa: E501\n"),
            "{out}"
        );
        let list = guards_list(&report.guards);
        assert_eq!(list.lines().filter(|l| l.starts_with("- ")).count(), 50);
        assert!(
            list.contains("\n2 more in `.jevgate/latest.json`.\n"),
            "{list}"
        );
    }
}
