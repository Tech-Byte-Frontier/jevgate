use crate::{
    options::{CheckArgs, ColorChoice, Format},
    schema::{FileResult, Finding, Report, Status, Strength},
};
use anyhow::Result;
use std::{
    collections::BTreeMap,
    io::{IsTerminal, Write},
    path::Path,
};

/// Consider findings shown by default; `--verbose` shows all.
const TOP_CONSIDER: usize = 10;

// Published Jev rate, checked 2026-09-18:
// https://typesafe.ai/blog/introducing-system-one-models-and-jev
pub const INPUT_USD_PER_MILLION: f64 = 0.042;
pub const PRICE_CHECKED: &str = "2026-09-18";

/// Estimated dollars for this invocation's paid input tokens, for a priced model.
pub fn estimated_usd(report: &Report) -> Option<f64> {
    usd(&report.requested_model, report.paid_input_tokens)
}

/// Estimated dollars for `tokens` input tokens of `model`, when it is priced.
fn usd(model: &str, tokens: u64) -> Option<f64> {
    (model == "jev-1.13.0").then(|| tokens as f64 * INPUT_USD_PER_MILLION / 1_000_000.0)
}

// ANSI select-graphic-rendition codes.
const BOLD: &str = "1";
const DIM: &str = "2";
const RED: &str = "31";
const CYAN: &str = "36";
const BOLD_RED: &str = "1;31";
const BOLD_GREEN: &str = "1;32";
const BOLD_YELLOW: &str = "1;33";

/// ANSI styles for agent output, or plain text.
#[derive(Clone, Copy)]
pub(crate) struct Style(bool);

impl Style {
    pub(crate) const PLAIN: Self = Self(false);

    /// `--color`, then NO_COLOR (set and not empty: off) and CLICOLOR_FORCE
    /// (set and not `0`: on), then whether stdout is a terminal that shows color.
    fn for_stdout(choice: ColorChoice) -> Self {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
        Self(match choice {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto if var("NO_COLOR").is_some() => false,
            ColorChoice::Auto if var("CLICOLOR_FORCE").is_some_and(|v| v != "0") => true,
            ColorChoice::Auto => std::io::stdout().is_terminal() && color_terminal(),
        })
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.0 {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

/// Whether the terminal shows ANSI color: not `TERM=dumb`, and on Windows
/// only Windows Terminal or a terminal that sets TERM, as the legacy console
/// prints the codes.
fn color_terminal() -> bool {
    let term = std::env::var_os("TERM");
    if cfg!(windows) {
        std::env::var_os("WT_SESSION").is_some() || term.is_some_and(|t| t != "dumb")
    } else {
        term.is_none_or(|t| t != "dumb")
    }
}

/// Write the report to stdout. A reader that closes the pipe early (as with
/// `| head`) ends the output without failing the run, so the exit code still
/// reflects the gate.
pub fn emit(report: &Report, args: &CheckArgs) -> Result<()> {
    let mut out = std::io::stdout().lock();
    let written = match args.output_format() {
        Format::Json => serde_json::to_writer_pretty(&mut out, report)
            .map_err(anyhow::Error::from)
            .and_then(|()| Ok(writeln!(out)?)),
        Format::Jsonl => serde_json::to_writer(&mut out, report)
            .map_err(anyhow::Error::from)
            .and_then(|()| Ok(writeln!(out)?)),
        Format::Agent => agent(
            &mut out,
            report,
            args.verbose,
            Style::for_stdout(args.color),
        ),
        Format::Github => crate::github::emit(&mut out, report, args),
    };
    match written {
        Err(error) if broken_pipe(&error) => Ok(()),
        other => other,
    }
}

fn broken_pipe(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
            || cause
                .downcast_ref::<serde_json::Error>()
                .and_then(|json| json.io_error_kind())
                == Some(std::io::ErrorKind::BrokenPipe)
    })
}

pub(crate) fn label(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn agent(
    out: &mut impl Write,
    report: &Report,
    verbose: bool,
    style: Style,
) -> Result<()> {
    emit_header(out, report, style)?;
    emit_findings(out, report, verbose, style)?;
    emit_summary(out, report)?;
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

/// Status, gate, scope and cost on one line; for a dry run, the planned
/// requests and the cost of those the cache does not answer.
pub(crate) fn headline(report: &Report) -> String {
    if report.dry_run {
        let stages = report.stages.values();
        let planned: u64 = stages.clone().map(|s| s.planned_requests).sum();
        let cached: u64 = stages.clone().map(|s| s.planned_cached).sum();
        let tokens: u64 = stages.map(|s| s.planned_tokens).sum();
        let cost = usd(&report.requested_model, tokens)
            .map_or(String::new(), |usd| format!(" · ~${usd:.4}"));
        return format!(
            "JevGate: dry run · {} files · {planned} first-pass requests, {cached} answered by the cache · ~{tokens} new input tokens{cost}; follow-ups depend on the answers",
            report.files.len()
        );
    }
    let gate = match &report.gate {
        Some(gate) if gate.passed => "gate passed".to_string(),
        Some(gate) => format!("gate failed: {}", gate.reasons.join("; ")),
        None => "gate not evaluated".to_string(),
    };
    let cost = estimated_usd(report).map_or(String::new(), |usd| format!(" · ~${usd:.4}"));
    format!(
        "JevGate: {} · {gate} · {} files · {} API requests · {} input tokens{cost}",
        report.status,
        report.files.len(),
        report.api_requests,
        report.paid_input_tokens
    )
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

/// Every finding with its file's path, highest rank first.
pub(crate) fn ranked(report: &Report) -> Vec<(&Path, &Finding)> {
    let mut findings: Vec<(&Path, &Finding)> = report
        .files
        .iter()
        .flat_map(|f| {
            f.findings
                .iter()
                .map(move |finding| (f.path.as_path(), finding))
        })
        .collect();
    findings.sort_by(|a, b| b.1.rank.total_cmp(&a.1.rank));
    findings
}

/// Every review, then the top-ranked considers (all with `verbose`). Notes
/// are listed only with `verbose`; otherwise just counted.
fn emit_findings(out: &mut impl Write, report: &Report, verbose: bool, style: Style) -> Result<()> {
    let findings = ranked(report);
    let of = |strength: Strength| -> Vec<(&Path, &Finding)> {
        findings
            .iter()
            .filter(|(_, f)| f.strength == strength)
            .copied()
            .collect()
    };
    let (review, consider, notes) = (
        of(Strength::Review),
        of(Strength::Consider),
        of(Strength::Note),
    );
    if !review.is_empty() {
        let heading = format!("Review ({}):", review.len());
        emit_section(out, &heading, BOLD_RED, &review, style)?;
    }
    if !consider.is_empty() {
        let shown = if verbose {
            consider.len()
        } else {
            TOP_CONSIDER
        };
        let more = if consider.len() > shown {
            format!(", top {shown}; --verbose shows all")
        } else {
            String::new()
        };
        let heading = format!("Consider ({}{more}):", consider.len());
        emit_section(
            out,
            &heading,
            BOLD_YELLOW,
            &consider[..shown.min(consider.len())],
            style,
        )?;
    }
    if notes.is_empty() {
        return Ok(());
    }
    if !verbose {
        writeln!(
            out,
            "\n{} optional note(s) on code that reads well as it is; --verbose shows them.",
            notes.len()
        )?;
        return Ok(());
    }
    let heading = format!("Notes ({}, optional):", notes.len());
    emit_section(out, &heading, BOLD, &notes, style)
}

/// A blank line, a heading, then its findings.
fn emit_section(
    out: &mut impl Write,
    heading: &str,
    code: &str,
    findings: &[(&Path, &Finding)],
    style: Style,
) -> Result<()> {
    writeln!(out, "\n{}", style.paint(code, heading))?;
    for (path, finding) in findings {
        emit_finding(out, path, finding, style)?;
    }
    Ok(())
}

/// Counts of undecided, unsent and failed files, and skip reasons.
fn emit_summary(out: &mut impl Write, report: &Report) -> Result<()> {
    let count = |status: Status| report.files.iter().filter(|f| f.status == status).count();
    let undecided = report
        .files
        .iter()
        .filter(|f| f.dimensions.values().any(|d| d.status == Status::Uncertain))
        .count();
    let summary = [
        (undecided, "with uncertain units"),
        (count(Status::NeedsContext), "need context"),
        (count(Status::Error), "failed"),
    ];
    let lines: Vec<_> = summary
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, text)| format!("{n} files {text}"))
        .collect();
    if !lines.is_empty() {
        writeln!(out, "\n{}.", lines.join(" · "))?;
    }
    let mut skipped = BTreeMap::<String, usize>::new();
    for file in report.files.iter().filter(|f| f.status == Status::Skipped) {
        *skipped
            .entry(file.error.clone().unwrap_or_else(|| "Skipped".into()))
            .or_default() += 1;
    }
    for (reason, n) in skipped {
        writeln!(out, "Skipped {n}: {reason}")?;
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

/// `path:line [rule] message`, then the next step; the location is bold and
/// the rule dim.
fn emit_finding(out: &mut impl Write, path: &Path, finding: &Finding, style: Style) -> Result<()> {
    let location = format!("{}:{}", path.display(), finding.line);
    let rule = format!(
        "[{}]{}",
        finding.rule,
        if finding.baselined {
            " (baselined)"
        } else {
            ""
        }
    );
    writeln!(
        out,
        "  {} {} {}",
        style.paint(BOLD, &location),
        style.paint(DIM, &rule),
        finding.message
    )?;
    writeln!(out, "    {} {}", style.paint(CYAN, "→"), finding.action)?;
    Ok(())
}

pub(super) fn emit_file(out: &mut impl Write, file: &FileResult) -> Result<()> {
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
    use crate::tests::finding;

    fn line(style: Style) -> String {
        let mut out = Vec::new();
        emit_finding(
            &mut out,
            Path::new("src/a.rs"),
            &finding(Strength::Review),
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
