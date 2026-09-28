use crate::{
    options::{CheckArgs, ColorChoice, Format},
    schema::{FileResult, Finding, Gating, Report, Scope, Status, Strength},
};
use anyhow::Result;
use std::{
    collections::BTreeMap,
    io::{IsTerminal, Write},
    path::Path,
};

/// Consider findings shown by default; `--verbose` shows all.
const TOP_CONSIDER: usize = 10;
/// Guards shown by default; `--verbose` shows all.
const TOP_GUARDS: usize = 10;
/// Says what guards are, after their count.
pub(crate) const GUARDS_HEADING: &str =
    "changes to the checks around this code, for a person to look at; they never fail the gate";

/// `n` and a noun, plural unless `n` is one: "1 finding", "2 findings".
pub fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// The headline's cost: estimated dollars, or unknown, never a guessed $0.
fn cost(usd: Option<f64>) -> String {
    usd.map_or(" · cost unknown".into(), |usd| format!(" · ~${usd:.4}"))
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
        Format::Sarif => crate::sarif::emit(&mut out, report),
        Format::Gitlab => crate::gitlab::emit(&mut out, report),
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
    if let Some(line) = measuring(report) {
        writeln!(out, "\n{line}")?;
    }
    emit_guards(out, report, verbose, style)?;
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

/// What a dry run plans: first-pass requests and their questions, those the
/// cache answers, and the estimated input tokens and dollars of what the
/// rest send: a request sends only the questions the cache lacks.
#[derive(serde::Serialize)]
pub(crate) struct Preview {
    pub requests: u64,
    pub cached: u64,
    pub questions: u64,
    pub cached_questions: u64,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd: Option<f64>,
}

pub(crate) fn preview(report: &Report) -> Preview {
    let total = |field: fn(&crate::schema::StageMetrics) -> u64| {
        report.stages.values().map(field).sum::<u64>()
    };
    let tokens = total(|s| s.planned_tokens);
    Preview {
        requests: total(|s| s.planned_requests),
        cached: total(|s| s.planned_cached),
        questions: total(|s| s.planned_questions),
        cached_questions: total(|s| s.planned_cached_questions),
        tokens,
        usd: crate::model::usd(&report.requested_model, tokens),
    }
}

/// Status, gate, scope and cost on one line; for a dry run, the planned
/// requests and questions and the cost of the questions the cache does not
/// answer.
pub(crate) fn headline(report: &Report) -> String {
    if report.dry_run {
        let Preview {
            requests,
            cached,
            questions,
            cached_questions,
            tokens,
            usd,
        } = preview(report);
        return format!(
            "JevGate: dry run · {} files{} · {requests} first-pass requests, {cached} answered by the cache · {questions} questions, {cached_questions} answered by the cache · ~{tokens} new input tokens{}; follow-ups depend on the answers",
            report.files.len(),
            since(report),
            cost(usd)
        );
    }
    let gate = match &report.gate {
        Some(gate) if gate.passed => "gate passed".to_string(),
        Some(gate) => format!("gate failed: {}", gate.reasons.join("; ")),
        None => "gate not evaluated".to_string(),
    };
    let cost = cost(report.estimated_usd);
    format!(
        "JevGate: {} · {gate} · {} files{} · {} API requests{} · {} input tokens{cost}",
        report.status,
        report.files.len(),
        since(report),
        report.api_requests,
        via(report),
        report.paid_input_tokens
    )
}

/// ` via OpenRouter` when a gateway answered, so a key found in the
/// environment never bills another account unseen; nothing for TypeSafe.
fn via(report: &Report) -> String {
    crate::provider::Provider::named(&report.provider)
        .filter(|provider| *provider != crate::provider::Provider::Typesafe)
        .map_or(String::new(), |provider| {
            format!(" via {}", provider.service().label)
        })
}

/// Characters of a commit id shown, as Git abbreviates it.
const SHORT_COMMIT: usize = 7;

/// With a base revision, what the check judged since it: ` · changed lines
/// since 1a2b3c4` or ` · whole files changed since 1a2b3c4`.
fn since(report: &Report) -> String {
    let Some(base) = &report.base_revision else {
        return String::new();
    };
    let judged = match report.scope {
        Scope::ChangedLines => "changed lines",
        Scope::WholeFiles => "whole files changed",
    };
    format!(
        " · {judged} since {}",
        base.get(..SHORT_COMMIT).unwrap_or(base)
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

/// Every finding with its file's path: those that fail the gate first, then
/// the rest by level, reviews first, each highest rank first. A capped list
/// never leaves out a failure for a finding that only warns, nor a review
/// still being measured for a higher-ranked consider. GitHub shows 10
/// warning annotations a step: in whole-repository runs of 94 corpus
/// projects with the default rules, 267 of 424 such reviews fell past the
/// tenth when ranked with considers, and 109 with reviews first, all in the
/// 10 projects holding more than ten of them.
pub(crate) fn failing_first(report: &Report) -> Vec<(&Path, &Finding)> {
    let mut findings = ranked(report);
    // A stable sort keeps the rank order within each part.
    findings.sort_by_key(|(_, f)| (!f.fails_gate(), std::cmp::Reverse(f.strength)));
    findings
}

/// Every review, then the top considers (all with `verbose`), those that
/// fail the gate first. Notes are listed only with `verbose`; otherwise just
/// counted.
fn emit_findings(out: &mut impl Write, report: &Report, verbose: bool, style: Style) -> Result<()> {
    let findings = failing_first(report);
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
        emit_considers(out, &consider, verbose, style)?;
    }
    if notes.is_empty() {
        return Ok(());
    }
    if !verbose {
        writeln!(
            out,
            "\n{} on code that reads well as it is; --verbose shows them.",
            count(notes.len(), "optional note")
        )?;
        return Ok(());
    }
    let heading = format!("Notes ({}, optional):", notes.len());
    emit_section(out, &heading, BOLD, &notes, style)
}

/// The top considers (all with `verbose`), under a heading that says how
/// many there are and which are shown.
fn emit_considers(
    out: &mut impl Write,
    consider: &[(&Path, &Finding)],
    verbose: bool,
    style: Style,
) -> Result<()> {
    let shown = if verbose {
        consider.len()
    } else {
        TOP_CONSIDER.min(consider.len())
    };
    let more = if consider.len() > shown {
        let order = if consider.iter().any(|(_, f)| f.fails_gate()) {
            ", those that fail the gate first"
        } else {
            ""
        };
        format!(", top {shown}{order}; --verbose shows all")
    } else {
        String::new()
    };
    let heading = format!("Consider ({}{more}):", consider.len());
    emit_section(out, &heading, BOLD_YELLOW, &consider[..shown], style)
}

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

/// Why reviews did not fail the gate when their rules and levels are still
/// being measured, with the considers beside them and how to make every
/// review fail it; none when no review is left out that way.
pub(crate) fn measuring(report: &Report) -> Option<String> {
    let left_out = |strength: Strength| {
        report
            .files
            .iter()
            .flat_map(|f| &f.findings)
            .filter(|f| f.strength == strength && f.gate == Some(Gating::Measuring))
            .count()
    };
    let (reviews, considers) = (left_out(Strength::Review), left_out(Strength::Consider));
    if reviews == 0 {
        return None;
    }
    let considers = if considers > 0 {
        format!(" and {}", count(considers, "consider"))
    } else {
        String::new()
    };
    Some(format!(
        "{}{considers} did not fail the gate: by default only rules and levels right at least {}% of the time on projects JevGate was never tuned on fail it, and theirs are still being measured. `jevgate rules` shows each one's precision; `--fail-on review` makes every review fail the gate.",
        count(reviews, "review"),
        crate::maturity::MIN_PERCENT_RIGHT
    ))
}

/// Why a finding still being measured does not fail the gate; none for any
/// other finding. Its claim already says how often its rule and level were
/// right.
pub(crate) fn measuring_note(finding: &Finding) -> Option<String> {
    (finding.gate == Some(Gating::Measuring)).then(|| {
        format!(
            "Does not fail the gate: by default only rules and levels right at least {}% of the time over at least {} labels on projects JevGate was never tuned on fail it.",
            crate::maturity::MIN_PERCENT_RIGHT,
            crate::maturity::MIN_LABELS
        )
    })
}

/// A finding's message, then how often findings of its rule and level were
/// right on projects JevGate was never tuned on, in place of the probability
/// of the answer that set its level: "… Right 87% of the time (23 labels)."
/// or "… Not yet measured."; a note's message alone.
pub(crate) fn claim(finding: &Finding, style: Style) -> String {
    let Some(labels) = finding.precision else {
        return finding.message.clone();
    };
    let words = crate::maturity::precision_in_words(&finding.rule, labels);
    let mut chars = words.chars();
    let sentence = chars.next().map_or_else(String::new, |first| {
        format!("{}{}.", first.to_uppercase(), chars.as_str())
    });
    format!("{} {}", finding.message, style.paint(DIM, &sentence))
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
    Ok(())
}

/// Each reason the files of `status` give, with how many give it: a run that
/// could not finish says why without --verbose, as `Failed 1: TypeSafe HTTP
/// 402 (credits exhausted; …)`, and the MCP tools, which return these lines,
/// can tell exhausted credits from a missing key.
pub(crate) fn reasons(report: &Report, status: Status) -> BTreeMap<&str, usize> {
    let unknown = if status == Status::Skipped {
        "Skipped"
    } else {
        "Not judged"
    };
    let mut reasons = BTreeMap::new();
    for file in report.files.iter().filter(|f| f.status == status) {
        *reasons
            .entry(file.error.as_deref().unwrap_or(unknown))
            .or_default() += 1;
    }
    reasons
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
fn emit_finding(out: &mut impl Write, path: &Path, finding: &Finding, style: Style) -> Result<()> {
    let location = format!("{}:{}", path.display(), finding.line);
    let accepted = match (&finding.suppressed, finding.baselined) {
        (_, true) => " (baselined)".to_string(),
        (Some(reason), false) => format!(" (allowed: {reason})"),
        (None, false) => String::new(),
    };
    let rule = format!("[{}]{accepted}", finding.rule);
    let fails = if finding.fails_gate() {
        format!("{} ", style.paint(RED, "(fails the gate)"))
    } else {
        String::new()
    };
    writeln!(
        out,
        "  {} {} {fails}{}",
        style.paint(BOLD, &location),
        style.paint(DIM, &rule),
        claim(finding, style)
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
