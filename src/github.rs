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
/// a note (an error when it fails the gate, else a warning), then the agent
/// text. The summary goes to `$GITHUB_STEP_SUMMARY` when the runner sets it.
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
    let shown: Vec<(&Path, &Finding)> = output::ranked(report)
        .into_iter()
        .filter(|(_, f)| f.strength != Strength::Note && !f.baselined)
        .collect();
    for (path, finding) in &shown {
        writeln!(
            out,
            "{}",
            annotation(path, finding, crate::gate::fails(finding, args))
        )?;
    }
    if let Some(file) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let written = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&file)
            .and_then(|mut f| f.write_all(summary(report, &shown, args).as_bytes()));
        if let Err(error) = written {
            note!("jevgate: cannot write the job summary: {error}");
        }
    }
    output::agent(out, report, args.verbose)
}

fn annotation(path: &Path, finding: &Finding, fails: bool) -> String {
    let end = finding
        .locations
        .iter()
        .find(|l| l.path == path && l.start_line == finding.line)
        .map_or(String::new(), |l| format!(",endLine={}", l.end_line));
    format!(
        "::{} file={},line={}{end},title={}::{}",
        if fails { "error" } else { "warning" },
        property(&path.to_string_lossy()),
        finding.line,
        property(&format!("JevGate {} [{}]", label(finding), finding.rule)),
        data(&format!("{}\n→ {}", finding.message, finding.action)),
    )
}

fn label(finding: &Finding) -> String {
    output::label(&finding.strength)
}

/// The Markdown job summary: the headline, then a table of findings.
fn summary(report: &Report, shown: &[(&Path, &Finding)], args: &CheckArgs) -> String {
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
            "- **{failed} file(s) not judged;** the annotations give each reason.\n\n"
        ));
    }
    if shown.is_empty() {
        text.push_str("No new review or consider findings.\n\n");
        return text;
    }
    text.push_str("| | Location | Rule | Finding |\n|---|---|---|---|\n");
    for (path, finding) in shown.iter().take(SUMMARY_ROWS) {
        let level = if crate::gate::fails(finding, args) {
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
    use crate::schema::Location;

    fn finding(strength: Strength) -> Finding {
        Finding {
            rule: "maintainability/shared-logic".into(),
            strength,
            line: 12,
            message: "Copies: 50% alike,\nsee `b`".into(),
            action: "Share one | implementation".into(),
            symbol: None,
            rule_version: String::new(),
            concern_probability: 0.9,
            locations: vec![Location {
                path: "src/a,b.rs".into(),
                start_line: 12,
                end_line: 20,
                symbol: None,
            }],
            quote: None,
            category: None,
            values: Vec::new(),
            fingerprint: String::new(),
            rank: 1.0,
            baselined: false,
        }
    }

    #[test]
    fn annotations_escape_commands_and_mark_what_fails_the_gate() {
        let line = annotation(Path::new("src/a,b.rs"), &finding(Strength::Review), true);
        assert_eq!(
            line,
            "::error file=src/a%2Cb.rs,line=12,endLine=20,title=JevGate review [maintainability/shared-logic]::Copies: 50%25 alike,%0Asee `b`%0A→ Share one | implementation"
        );
        assert!(!line.contains('\n'));
        let consider = annotation(Path::new("x.rs"), &finding(Strength::Consider), false);
        assert!(consider.starts_with("::warning file=x.rs,line=12,title="));
    }

    #[test]
    fn summary_rows_stay_on_one_line_and_bold_gate_failures() {
        let args = crate::tests::args();
        let review = finding(Strength::Review);
        let consider = finding(Strength::Consider);
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
        let text = summary(&report, &[(path, &review), (path, &consider)], &args);
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
