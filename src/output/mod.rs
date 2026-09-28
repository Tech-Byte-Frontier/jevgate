//! Writing a report to stdout in its format, and what every format says
//! alike: the headline, findings in rank order with how often their rule
//! and level were right, why reviews did not fail the gate, and the units
//! left out. The agent text, the default format, is in `agent`.
use crate::{
    options::{CheckArgs, ColorChoice, Format},
    schema::{Finding, Gating, LeftOut, Report, Scope, Status, Strength},
};
use anyhow::Result;
use std::{
    collections::BTreeMap,
    io::{IsTerminal, Write},
    path::Path,
};

mod agent;
pub(crate) use agent::agent;

/// Says what guards are, after their count.
pub(crate) const GUARDS_HEADING: &str =
    "changes to the checks around this code, for a person to look at; they never fail the gate";

/// `n` and a noun, plural unless `n` is one: "1 finding", "2 findings".
pub fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// "a", "a and b", "a, b and c".
pub(crate) fn join(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The headline's cost: estimated dollars, or unknown, never a guessed $0.
pub(crate) fn cost(usd: Option<f64>) -> String {
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
        Format::Sarif => crate::sarif::emit(&mut out, report, args.questions),
        Format::Gitlab => crate::gitlab::emit(&mut out, report),
    };
    match written {
        Err(error) if broken_pipe(&error) => Ok(()),
        other => other,
    }
}

pub(crate) fn broken_pipe(error: &anyhow::Error) -> bool {
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
    crate::provider::Provider::named(&report.provider).map_or(String::new(), through)
}

/// ` via <gateway>` for a gateway's key; nothing for TypeSafe's.
pub(crate) fn through(provider: crate::provider::Provider) -> String {
    if provider == crate::provider::Provider::Typesafe {
        String::new()
    } else {
        format!(" via {}", provider.service().label)
    }
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

/// Why reviews did not fail the gate: their rules and levels are still
/// being measured, or their files' languages are in preview. Each reason
/// comes with the considers beside its reviews and how to make every review
/// fail the gate; none when no review is left out either way.
pub(crate) fn measuring(report: &Report) -> Option<String> {
    let (preview, measured): (Vec<_>, Vec<_>) = report
        .files
        .iter()
        .flat_map(|f| {
            f.findings
                .iter()
                .map(move |finding| (f.path.as_path(), finding))
        })
        .filter(|(_, f)| f.gate == Some(Gating::Measuring))
        .partition(|(path, f)| crate::maturity::preview_language(path, &f.rule).is_some());
    let measured = reviews_and_considers(&measured).map(|findings| format!(
        "{findings} did not fail the gate: by default only rules and levels right at least {}% of the time on projects JevGate was never tuned on fail it, and theirs are still being measured. `jevgate rules` shows each one's precision; `--fail-on review` makes every review fail the gate.",
        crate::maturity::MIN_PERCENT_RIGHT
    ));
    let preview = reviews_and_considers(&preview).map(|findings| {
        let mut languages: Vec<String> = preview
            .iter()
            .filter_map(|(path, f)| crate::maturity::preview_language(path, &f.rule))
            .map(str::to_string)
            .collect();
        languages.sort();
        languages.dedup();
        let which = match languages.as_slice() {
            [one] => format!("{one} is"),
            _ => "those languages are".into(),
        };
        format!(
            "{findings} in {} files did not fail the gate: {which} in preview, and by default JevGate's own rules never fail it there. `--fail-on review` makes every review fail the gate.",
            join(&languages)
        )
    });
    let reasons: Vec<String> = [measured, preview].into_iter().flatten().collect();
    (!reasons.is_empty()).then(|| reasons.join("\n\n"))
}

/// "2 reviews and 1 consider" of `findings`; none without a review, which
/// alone would have failed the default gate.
fn reviews_and_considers(findings: &[(&Path, &Finding)]) -> Option<String> {
    let of = |strength: Strength| {
        findings
            .iter()
            .filter(|(_, f)| f.strength == strength)
            .count()
    };
    let (reviews, considers) = (of(Strength::Review), of(Strength::Consider));
    (reviews > 0).then(|| match considers {
        0 => count(reviews, "review"),
        _ => format!(
            "{} and {}",
            count(reviews, "review"),
            count(considers, "consider")
        ),
    })
}

/// Why a finding in `path` still being measured does not fail the gate:
/// its rule and level are not yet mature, or its file's language is in
/// preview; none for any other finding. Its claim already says how often its
/// rule and level were right.
pub(crate) fn measuring_note(path: &Path, finding: &Finding) -> Option<String> {
    (finding.gate == Some(Gating::Measuring)).then(|| {
        match crate::maturity::preview_language(path, &finding.rule) {
            Some(language) => format!(
                "Does not fail the gate: {language} is in preview, and by default JevGate's own rules never fail it there."
            ),
            None => format!(
                "Does not fail the gate: by default only rules and levels right at least {}% of the time over at least {} labels on projects JevGate was never tuned on fail it.",
                crate::maturity::MIN_PERCENT_RIGHT,
                crate::maturity::MIN_LABELS
            ),
        }
    })
}

/// A finding's message, then how often findings of its rule and level were
/// right on projects JevGate was never tuned on, in place of the probability
/// of the answer that set its level: "… Right 87% of the time (23 labels)."
/// or "… Not yet measured."; in a preview language, its own: "… Not yet
/// measured in Kotlin."; a note's message alone.
pub(crate) fn claim(path: &Path, finding: &Finding, style: Style) -> String {
    claimed(&finding.message, path, finding, style)
}

/// `why`, the finding's message or a cut of it, then how often findings of
/// its rule and level were right, as [`claim`] ends it: the agent hook cuts
/// a long message, never the sentence a reader weighs the finding by.
pub(crate) fn claimed(why: &str, path: &Path, finding: &Finding, style: Style) -> String {
    let Some(labels) = finding.precision else {
        return why.to_string();
    };
    let language = crate::maturity::preview_language(path, &finding.rule);
    let words = crate::maturity::precision_in_words(&finding.rule, labels, language);
    let mut chars = words.chars();
    let sentence = chars.next().map_or_else(String::new, |first| {
        format!("{}{}.", first.to_uppercase(), chars.as_str())
    });
    format!("{why} {}", style.paint(DIM, &sentence))
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

/// The units the parser could not read in judged files, each with its file.
pub(crate) fn left_out(report: &Report) -> Vec<(&Path, &LeftOut)> {
    report
        .files
        .iter()
        .flat_map(|f| f.left_out.iter().map(move |l| (f.path.as_path(), l)))
        .collect()
}

/// A unit left out, as a line names it: `name`, `lines 4-9` or `line 4`.
pub(crate) fn left_out_unit(entry: &LeftOut) -> String {
    match (entry.unit.as_str(), entry.end_line > entry.start_line) {
        ("", true) => format!("lines {}-{}", entry.start_line, entry.end_line),
        ("", false) => format!("line {}", entry.start_line),
        (name, _) => name.to_string(),
    }
}

/// `path:line unit: reason`, the line that names a unit left out.
pub(crate) fn left_out_line(path: &Path, entry: &LeftOut) -> String {
    format!(
        "{}:{} {}: {}",
        path.display(),
        entry.start_line,
        left_out_unit(entry),
        entry.reason
    )
}
