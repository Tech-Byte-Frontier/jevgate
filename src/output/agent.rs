//! The agent text, the default format, for people and coding agents: the
//! headline, then reviews, the top considers and custom questions' notes,
//! guards, why files failed or were skipped, preview languages, units left
//! out and the instructions loaded at session start; with `--verbose`, every
//! note and each file's answers.
use super::{
    BOLD, BOLD_GREEN, BOLD_RED, CYAN, DIM, GUARDS_HEADING, RED, Style, claim, count, failing_first,
    headline, label, left_out, left_out_line, measuring, reasons,
};
use crate::schema::{FileResult, Finding, Report, Scope, Status};
use anyhow::Result;
use std::{collections::BTreeMap, io::Write, path::Path};

/// Consider findings shown by default; `--verbose` shows all.
const TOP_REVIEWS: usize = 10;
/// Guards shown by default; `--verbose` shows all.
const TOP_GUARDS: usize = 10;
/// Units left out over syntax errors shown by default; `--verbose` shows all.
const TOP_LEFT_OUT: usize = 10;

pub(crate) fn agent(
    out: &mut impl Write,
    report: &Report,
    verbose: bool,
    style: Style,
) -> Result<()> {
    emit_header(out, report, style)?;
    emit_findings(out, report, verbose, style)?;
    if let Some(line) = measuring(report) {
        writeln!(out, "\n{line}")?;
    }
    emit_guards(out, report, verbose, style)?;
    emit_summary(out, report)?;
    emit_preview(out, report)?;
    emit_left_out(out, report, verbose)?;
    if let Some(load) = &report.context_load {
        emit_context_load(out, load)?;
    }
    if verbose {
        writeln!(out)?;
        for file in &report.files {
            emit_file(out, file)?;
        }
    }
    Ok(())
}

/// The headline, green when the gate passed and red when it failed, then run errors.
fn emit_header(out: &mut impl Write, report: &Report, style: Style) -> Result<()> {
    let code = match &report.gate {
        Some(gate) if gate.passed => BOLD_GREEN,
        Some(_) => BOLD_RED,
        None => BOLD,
    };
    writeln!(out, "{}", style.paint(code, &headline(report)))?;
    for error in &report.errors {
        writeln!(out, "{} {error}", style.paint(RED, "Error:"))?;
    }
    Ok(())
}

/// One level: every finding is a review, those that fail the gate first.
/// All that fail it are listed, then the top ten others by rank; `verbose`
/// lists every one.
fn emit_findings(out: &mut impl Write, report: &Report, verbose: bool, style: Style) -> Result<()> {
    let findings = failing_first(report);
    if findings.is_empty() {
        return Ok(());
    }
    let failing = findings.iter().filter(|(_, f)| f.fails_gate()).count();
    let shown = if verbose {
        findings.len()
    } else {
        findings.len().min(failing + TOP_REVIEWS)
    };
    let more = if findings.len() > shown {
        format!(", top {shown}, those that fail the gate first; --verbose shows all")
    } else {
        String::new()
    };
    let heading = format!("Review ({}{more}):", findings.len());
    let gate = Gate(report.gate.is_some());
    emit_section(out, (&heading, BOLD_RED), &findings[..shown], gate, style)
}

/// Whether the run evaluated the gate: a finding of a run that did not, as
/// one left incomplete, would fail it rather than failing it.
#[derive(Clone, Copy)]
struct Gate(bool);

/// What the change does to the checks around the code, one line each: the
/// first ten, or all with `verbose`.
fn emit_guards(out: &mut impl Write, report: &Report, verbose: bool, style: Style) -> Result<()> {
    let guards = &report.guards;
    if guards.is_empty() {
        return Ok(());
    }
    let shown = if verbose {
        guards.len()
    } else {
        TOP_GUARDS.min(guards.len())
    };
    let more = if guards.len() > shown {
        format!(", first {shown}; --verbose shows all")
    } else {
        String::new()
    };
    let heading = format!("Guards ({}{more}): {GUARDS_HEADING}", guards.len());
    writeln!(out, "\n{}", style.paint(BOLD, &heading))?;
    for guard in &guards[..shown] {
        writeln!(out, "  {}", guard.describe())?;
    }
    Ok(())
}

/// A blank line, a heading in its color, then its findings.
fn emit_section(
    out: &mut impl Write,
    (heading, code): (&str, &str),
    findings: &[(&Path, &Finding)],
    gate: Gate,
    style: Style,
) -> Result<()> {
    writeln!(out, "\n{}", style.paint(code, heading))?;
    for (path, finding) in findings {
        emit_finding(out, path, finding, gate, style)?;
    }
    Ok(())
}

/// Counts of undecided, unsent and failed files, then why files failed and
/// why they were skipped.
fn emit_summary(out: &mut impl Write, report: &Report) -> Result<()> {
    let files = |status: Status| report.files.iter().filter(|f| f.status == status).count();
    let undecided = report
        .files
        .iter()
        .filter(|f| f.dimensions.values().any(|d| d.status == Status::Uncertain))
        .count();
    let summary = [
        (undecided, "with uncertain units", "with uncertain units"),
        (files(Status::NeedsContext), "needs context", "need context"),
        (files(Status::Error), "failed", "failed"),
    ];
    let lines: Vec<_> = summary
        .iter()
        .filter(|(n, ..)| *n > 0)
        .map(|&(n, one, many)| format!("{} {}", count(n, "file"), if n == 1 { one } else { many }))
        .collect();
    if !lines.is_empty() {
        writeln!(out, "\n{}.", lines.join(" · "))?;
    }
    for (label, status) in [("Failed", Status::Error), ("Skipped", Status::Skipped)] {
        for (reason, n) in reasons(report, status) {
            writeln!(out, "{label} {n}: {reason}")?;
        }
    }
    emit_capped(out, report)
}

/// Custom questions that reached their cap of units a run, with how many
/// units each left unasked.
fn emit_capped(out: &mut impl Write, report: &Report) -> Result<()> {
    let mut omitted = BTreeMap::<&str, usize>::new();
    for file in &report.files {
        for (rule, dimension) in &file.dimensions {
            if crate::catalog::custom(rule) && dimension.units.omitted > 0 {
                *omitted.entry(rule).or_default() += dimension.units.omitted;
            }
        }
    }
    for (rule, n) in omitted {
        writeln!(
            out,
            "{rule} left {} unasked: a question asks about at most {} units a run; narrow its paths.",
            count(n, "unit"),
            crate::units::MAX_CUSTOM_UNITS
        )?;
    }
    Ok(())
}

/// The files of the preview languages (`analysis::generic`) and what reads
/// them, by language: four rules and any custom question read their code,
/// none their test files yet, and the rules' findings never fail the
/// default gate. Without it, a `--rule security` check of a Kotlin project
/// passed with no word that security does not read Kotlin.
fn emit_preview(out: &mut impl Write, report: &Report) -> Result<()> {
    // Per language: files read, and test files not judged.
    let mut languages = BTreeMap::<&str, (usize, usize)>::new();
    for file in &report.files {
        let class = match &file.classification {
            Some(class) if file.status != Status::Skipped => class,
            _ => continue,
        };
        if crate::analysis::generic::of(&file.path).is_none() {
            continue;
        }
        let counts = languages.entry(class.language.as_str()).or_default();
        if class.kind == crate::file_kind::TESTS {
            counts.1 += 1;
        } else {
            counts.0 += 1;
        }
    }
    if languages.is_empty() {
        return Ok(());
    }
    let listed: Vec<String> = languages
        .iter()
        .map(|(language, &(read, tests))| match (read, tests) {
            (read, 0) => format!("{language} ({})", count(read, "file")),
            (0, tests) => format!("{language} ({} not judged yet)", count(tests, "test file")),
            (read, tests) => format!(
                "{language} ({}; {} not judged yet)",
                count(read, "file"),
                count(tests, "test file")
            ),
        })
        .collect();
    let readers = if report.rules.iter().any(|rule| crate::catalog::custom(rule)) {
        "function simplification, file organization, shared logic, comments and custom questions, whose built-in rules' findings"
    } else {
        "function simplification, file organization, shared logic and comments, whose findings"
    };
    writeln!(
        out,
        "\nPreview languages, read only by {readers} never fail the default gate: {}.",
        listed.join(", ")
    )?;
    Ok(())
}

/// The units the parser could not read in judged files, one line each
/// (`path:line unit: reason`), the first `TOP_LEFT_OUT` unless `verbose`.
fn emit_left_out(out: &mut impl Write, report: &Report, verbose: bool) -> Result<()> {
    let entries = left_out(report);
    if entries.is_empty() {
        return Ok(());
    }
    let files = report.files.iter().filter(|f| !f.left_out.is_empty());
    let judged = match report.scope {
        Scope::ChangedLines => "the code the change touched, the rest of it judged",
        Scope::WholeFiles => "the code, the rest of each file judged",
    };
    writeln!(
        out,
        "\nLeft out where the parser could not read {judged}: {} in {}.",
        count(entries.len(), "unit"),
        count(files.count(), "file")
    )?;
    let shown = if verbose { entries.len() } else { TOP_LEFT_OUT };
    for (path, entry) in entries.iter().take(shown) {
        writeln!(out, "  {}", left_out_line(path, entry))?;
    }
    if entries.len() > shown {
        writeln!(
            out,
            "  … {} more; --verbose shows all.",
            entries.len() - shown
        )?;
    }
    Ok(())
}

/// Estimated tokens each harness loads at session start, then loading facts.
fn emit_context_load(out: &mut impl Write, load: &crate::docs::load::ContextLoad) -> Result<()> {
    if load.harnesses.is_empty() {
        return Ok(());
    }
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let harnesses: Vec<String> = load
        .harnesses
        .iter()
        .map(|h| {
            let later = if h.on_demand_files > 0 {
                format!(", +{} on demand", h.on_demand_files)
            } else {
                String::new()
            };
            let files = h.files.len();
            format!(
                "{} ~{} ({files} file{}{later})",
                h.harness,
                h.estimated_tokens,
                plural(files)
            )
        })
        .collect();
    writeln!(
        out,
        "\nInstructions loaded at session start (estimated tokens): {}",
        harnesses.join(" · ")
    )?;
    for fact in &load.facts {
        writeln!(
            out,
            "  {}:{} {}",
            fact.path.display(),
            fact.line,
            fact.message
        )?;
    }
    Ok(())
}

/// `path:line [rule] message` and how often such findings were right, then
/// the next step; the location is bold, the rule and the share right dim,
/// and a finding that fails the gate says so in red.
fn emit_finding(
    out: &mut impl Write,
    path: &Path,
    finding: &Finding,
    Gate(evaluated): Gate,
    style: Style,
) -> Result<()> {
    let location = format!("{}:{}", path.display(), finding.line);
    let accepted = match (&finding.suppressed, finding.baselined) {
        (_, true) => " (baselined)".to_string(),
        (Some(reason), false) => format!(" (allowed: {reason})"),
        (None, false) => String::new(),
    };
    let rule = format!("[{}]{accepted}", finding.rule);
    let fails = if finding.fails_gate() {
        let label = if evaluated {
            "(fails the gate)"
        } else {
            "(would fail the gate)"
        };
        format!("{} ", style.paint(RED, label))
    } else {
        String::new()
    };
    writeln!(
        out,
        "  {} {} {fails}{}",
        style.paint(BOLD, &location),
        style.paint(DIM, &rule),
        claim(path, finding, style)
    )?;
    writeln!(out, "    {} {}", style.paint(CYAN, "→"), finding.action)?;
    Ok(())
}

fn emit_file(out: &mut impl Write, file: &FileResult) -> Result<()> {
    writeln!(out, "{} [{}]", file.path.display(), label(&file.status))?;
    if let Some(error) = &file.error {
        writeln!(out, "  {error}")?;
    } else if let Some(classification) = &file.classification
        && !classification.reason.is_empty()
    {
        writeln!(out, "  {}", classification.reason)?;
    }
    for (name, d) in &file.dimensions {
        writeln!(
            out,
            "  {name}: {} · concern {:.0}% · {}",
            label(&d.status),
            d.concern_probability * 100.0,
            d.decision_basis
        )?;
        for unit in &d.undecided {
            let values = if unit.values.is_empty() {
                String::new()
            } else {
                format!(" ({})", unit.values.join(", "))
            };
            writeln!(
                out,
                "    undecided: {} (line {}) · {}{values}",
                unit.unit,
                unit.line,
                unit.questions.join(", ")
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{options::ColorChoice, schema::Strength, tests::finding};

    fn line(style: Style) -> String {
        let mut out = Vec::new();
        emit_finding(
            &mut out,
            Path::new("src/a.rs"),
            &finding(Strength::Review),
            Gate(true),
            style,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn plain_findings_carry_no_escape_codes() {
        assert_eq!(
            line(Style::PLAIN),
            "  src/a.rs:12 [maintainability/shared-logic] Copies: 50% alike,\nsee `b`\n    → Share one | implementation\n"
        );
        assert!(!Style::for_stdout(ColorChoice::Never).0);
    }

    #[test]
    fn the_summary_says_why_files_failed() {
        let project = crate::tests::Project::new();
        project.write("a.rs", &crate::tests::function("a"));
        project.write("b.rs", &crate::tests::function("b"));
        let mut mock = crate::tests::Mock {
            malformed: true,
            ..Default::default()
        };
        let report = crate::tests::run(&project, &crate::tests::args(), &mut mock);
        let mut out = Vec::new();
        agent(&mut out, &report, false, Style::PLAIN).unwrap();
        let text = String::from_utf8(out).unwrap();
        let reason = report.files[0].error.as_deref().unwrap();
        assert!(
            text.contains(&format!("\n2 files failed.\nFailed 2: {reason}\n")),
            "{text}"
        );
    }

    #[test]
    fn colored_findings_bold_the_location_and_dim_the_rule() {
        assert!(Style::for_stdout(ColorChoice::Always).0);
        let text = line(Style(true));
        assert!(
            text.starts_with(
                "  \x1b[1msrc/a.rs:12\x1b[0m \x1b[2m[maintainability/shared-logic]\x1b[0m Copies"
            ),
            "{text:?}"
        );
        assert!(text.contains("\x1b[36m→\x1b[0m Share one"));
    }
}
