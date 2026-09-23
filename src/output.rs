use crate::{
    options::Format,
    schema::{FileResult, Finding, Report, Status, Strength},
};
use anyhow::Result;
use std::{collections::BTreeMap, io::Write, path::Path};

/// Consider findings shown by default; `--verbose` shows all.
const TOP_CONSIDER: usize = 10;

// Published Jev rate, checked 2026-09-18:
// https://typesafe.ai/blog/introducing-system-one-models-and-jev
pub const INPUT_USD_PER_MILLION: f64 = 0.042;
pub const PRICE_CHECKED: &str = "2026-09-18";

/// Estimated dollars for this invocation's paid input tokens, for a priced model.
pub fn estimated_usd(report: &Report) -> Option<f64> {
    (report.requested_model == "jev-1.13.0")
        .then(|| report.paid_input_tokens as f64 * INPUT_USD_PER_MILLION / 1_000_000.0)
}

/// Write the report to stdout. A reader that closes the pipe early (as with
/// `| head`) ends the output without failing the run, so the exit code still
/// reflects the gate.
pub fn emit(report: &Report, format: Format, verbose: bool) -> Result<()> {
    let mut out = std::io::stdout().lock();
    let written = match format {
        Format::Json => serde_json::to_writer_pretty(&mut out, report)
            .map_err(anyhow::Error::from)
            .and_then(|()| Ok(writeln!(out)?)),
        Format::Jsonl => serde_json::to_writer(&mut out, report)
            .map_err(anyhow::Error::from)
            .and_then(|()| Ok(writeln!(out)?)),
        Format::Agent => agent(&mut out, report, verbose),
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

fn label(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn agent(out: &mut impl Write, report: &Report, verbose: bool) -> Result<()> {
    emit_header(out, report)?;
    emit_findings(out, report, verbose)?;
    emit_summary(out, report)?;
    if verbose {
        writeln!(out)?;
        for file in &report.files {
            emit_file(out, file)?;
        }
    }
    Ok(())
}

/// One line: status, gate, scope and cost, then run errors.
fn emit_header(out: &mut impl Write, report: &Report) -> Result<()> {
    let gate = match &report.gate {
        Some(gate) if gate.passed => "gate passed".to_string(),
        Some(gate) => format!("gate failed: {}", gate.reasons.join("; ")),
        None => "gate not evaluated".to_string(),
    };
    let cost = estimated_usd(report).map_or(String::new(), |usd| format!(" · ~${usd:.4}"));
    writeln!(
        out,
        "JevGate: {} · {gate} · {} files · {} API requests · {} input tokens{cost}",
        report.status,
        report.files.len(),
        report.api_requests,
        report.paid_input_tokens
    )?;
    for error in &report.errors {
        writeln!(out, "Error: {error}")?;
    }
    Ok(())
}

/// Every review, then the top-ranked considers (all with `verbose`). Notes
/// are listed only with `verbose`; otherwise just counted.
fn emit_findings(out: &mut impl Write, report: &Report, verbose: bool) -> Result<()> {
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
        writeln!(out, "\nReview ({}):", review.len())?;
        for (path, finding) in &review {
            emit_finding(out, path, finding)?;
        }
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
        writeln!(out, "\nConsider ({}{more}):", consider.len())?;
        for (path, finding) in consider.iter().take(shown) {
            emit_finding(out, path, finding)?;
        }
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
    writeln!(out, "\nNotes ({}, optional):", notes.len())?;
    for (path, finding) in &notes {
        emit_finding(out, path, finding)?;
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

fn emit_finding(out: &mut impl Write, path: &Path, finding: &Finding) -> Result<()> {
    writeln!(
        out,
        "  {}:{} [{}]{} {}",
        path.display(),
        finding.line,
        finding.rule,
        if finding.baselined {
            " (baselined)"
        } else {
            ""
        },
        finding.message
    )?;
    writeln!(out, "    → {}", finding.action)?;
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
            writeln!(
                out,
                "    undecided: {} (line {}) · {}",
                unit.unit,
                unit.line,
                unit.questions.join(", ")
            )?;
        }
    }
    Ok(())
}
