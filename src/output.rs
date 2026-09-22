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

pub fn emit(report: &Report, format: Format, verbose: bool) -> Result<()> {
    let mut out = std::io::stdout().lock();
    match format {
        Format::Json => {
            serde_json::to_writer_pretty(&mut out, report)?;
            writeln!(out)?;
        }
        Format::Jsonl => {
            serde_json::to_writer(&mut out, report)?;
            writeln!(out)?;
        }
        Format::Agent => agent(&mut out, report, verbose)?,
    }
    Ok(())
}

fn label(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn agent(out: &mut impl Write, report: &Report, verbose: bool) -> Result<()> {
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
    let review: Vec<_> = findings
        .iter()
        .filter(|(_, f)| f.strength == Strength::Review)
        .collect();
    let consider: Vec<_> = findings
        .iter()
        .filter(|(_, f)| f.strength == Strength::Consider)
        .collect();
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
        writeln!(
            out,
            "\nConsider ({}{}):",
            consider.len(),
            if consider.len() > shown {
                format!(", top {shown}; --verbose shows all")
            } else {
                String::new()
            }
        )?;
        for (path, finding) in consider.iter().take(shown) {
            emit_finding(out, path, finding)?;
        }
    }
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
    if verbose {
        writeln!(out)?;
        for file in &report.files {
            emit_file(out, file)?;
        }
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
    }
    Ok(())
}
