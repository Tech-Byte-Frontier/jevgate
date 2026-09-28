//! What `jevgate rules propose` prints: a table of the proposals it wrote,
//! the proposals as `[[question]]` tables, every line as JSON, or a dry
//! run's plan and price.
use super::{
    PROPOSALS,
    ask::Answered,
    ask::{Plan, Price},
    files::{File, Skipped},
    proposal::{Candidate, Status, THRESHOLD, TOOL_CHECKED, proposed, rule, slashed},
};
use crate::output::count;
use anyhow::Result;
use serde_json::{Value, json};

/// Characters of a rule shown in the table.
const RULE_CHARS: usize = 60;

/// The requests one run sent and what they cost.
pub struct Usage {
    pub requests: u32,
    pub tokens: u64,
    /// Dollars, priced by the model that answered each request; none when
    /// unknown.
    pub usd: Option<f64>,
}

/// The files read, one line: `Read 2 instruction files, 41 lines: AGENTS.md, web/CLAUDE.md`.
fn read_line(files: &[File]) -> String {
    let lines: usize = files.iter().map(|f| f.lines.len()).sum();
    let names: Vec<String> = files.iter().map(|f| slashed(&f.path)).collect();
    if files.is_empty() {
        return "No agent instruction file found; name one, such as `jevgate rules propose CONTRIBUTING.md`.".into();
    }
    format!(
        "Read {}, {}: {}",
        count(files.len(), "file"),
        count(lines, "line"),
        names.join(", ")
    )
}

/// Files left out, one line each.
fn skipped_lines(skipped: &[Skipped]) -> Vec<String> {
    skipped
        .iter()
        .map(|s| format!("Not read: {} ({})", slashed(&s.path), s.reason))
        .collect()
}

/// A dry run's plan: `jevgate rules propose --dry-run`.
pub fn dry_run(
    files: &[File],
    skipped: &[Skipped],
    (plan, price): (&Plan, &Price),
    model: &str,
) -> String {
    let lines: usize = files.iter().map(|f| f.lines.len()).sum();
    let cost = crate::output::cost(crate::model::usd(model, price.tokens));
    let mut out = vec![format!(
        "JevGate: dry run · {} · {} · {} requests, {} answered by the cache · ~{} new input tokens{cost}; what would check each rule is asked after the answers",
        count(files.len(), "file"),
        count(lines, "line"),
        plan.requests.len(),
        price.cached,
        price.tokens
    )];
    if files.is_empty() {
        out.push(read_line(files));
    }
    out.extend(skipped_lines(skipped));
    out.join("\n")
}

/// A dry run as JSON, with the request bodies when `requests` is set.
pub fn dry_run_json(
    files: &[File],
    skipped: &[Skipped],
    (plan, price): (&Plan, &Price),
    requests: bool,
) -> Result<String> {
    let mut value = json!({
        "dry_run": true,
        "files": files_json(files),
        "skipped": skipped,
        "candidates": files.iter().flat_map(|file| file.lines.iter().map(move |line| line_json(file, line))).collect::<Vec<_>>(),
        "planned_requests": price.requests,
        "planned_cached": price.cached,
        "planned_tokens": price.tokens,
    });
    if requests {
        value["requests"] = plan
            .requests
            .iter()
            .map(|r| crate::requests::provider_request(r).into_owned())
            .collect();
    }
    Ok(serde_json::to_string_pretty(&value)?)
}

/// The proposals written, as a table, what was not proposed, and why a
/// line went unanswered.
pub fn table(
    files: &[File],
    skipped: &[Skipped],
    candidates: &[Candidate<'_>],
    (usage, errors): (&Usage, &[String]),
) -> String {
    let mut out = vec![read_line(files)];
    out.extend(skipped_lines(skipped));
    let new: Vec<&Candidate> = with_status(candidates, Status::New);
    let answered = candidates.iter().any(|c| c.answered.is_some());
    if !new.is_empty() {
        out.push(String::new());
        out.push(format!(
            "Proposed {} in {PROPOSALS}/:",
            count(new.len(), "question")
        ));
        out.extend(proposal_rows(&new));
    } else if answered {
        out.push(String::new());
        out.push("No new proposals.".into());
    }
    out.push(String::new());
    out.extend(not_proposed(candidates, errors));
    if !new.is_empty() {
        out.push("Edit a proposal, then accept it: jevgate rules accept ID".into());
    }
    out.push(usage_line(usage));
    out.join("\n")
}

/// Rows of id, unit, file and line, and the start of the rule.
fn proposal_rows(new: &[&Candidate<'_>]) -> Vec<String> {
    let rows: Vec<(&str, &str, &str, String)> = new
        .iter()
        .filter_map(|c| {
            let p = c.proposal.as_ref()?;
            let rule = shown(&c.line.text);
            Some((p.id.as_str(), p.unit.noun(), p.citation.as_str(), rule))
        })
        .collect();
    let id = rows.iter().map(|r| r.0.len()).max().unwrap_or(0).max(2);
    let unit = rows.iter().map(|r| r.1.len()).max().unwrap_or(0).max(4);
    let from = rows.iter().map(|r| r.2.len()).max().unwrap_or(0).max(4);
    let mut lines = vec![format!(
        "  {:id$}  {:unit$}  {:from$}  RULE",
        "ID", "UNIT", "FROM"
    )];
    for (i, u, f, rule) in rows {
        lines.push(format!("  {i:id$}  {u:unit$}  {f:from$}  {rule}"));
    }
    lines
}

/// A rule cut to [`RULE_CHARS`] for the table.
fn shown(text: &str) -> String {
    if text.chars().count() <= RULE_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(RULE_CHARS - 1).collect();
    format!("{}…", head.trim_end())
}

fn with_status<'c, 'a>(candidates: &'c [Candidate<'a>], status: Status) -> Vec<&'c Candidate<'a>> {
    candidates
        .iter()
        .filter(|c| c.proposal.as_ref().is_some_and(|p| p.status == status))
        .collect()
}

/// The lines left unproposed, and why, in a few sentences.
fn not_proposed(candidates: &[Candidate<'_>], errors: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let saved: Vec<String> = [
        (Status::Kept, "already proposed and kept as it is"),
        (Status::Accepted, "already a question"),
        (Status::Repeated, "the same rule as a line above"),
    ]
    .iter()
    .filter_map(|(status, what)| {
        let n = with_status(candidates, *status).len();
        (n > 0).then(|| format!("{} {what}", count(n, "line")))
    })
    .collect();
    if !saved.is_empty() {
        out.push(format!("Not written: {}.", saved.join("; ")));
    }
    let answered: Vec<&Answered> = candidates
        .iter()
        .filter_map(|c| c.answered.as_ref())
        .filter(|a| !rule(a) || a.checkers.is_some())
        .collect();
    let no_rule = answered.iter().filter(|a| !rule(a)).count();
    let tool = answered.iter().filter(|a| rule(a) && !proposed(a)).count();
    if no_rule > 0 {
        out.push(format!(
            "No rule to check on the code at {THRESHOLD:.2}: {} of {}.",
            count(no_rule, "line"),
            answered.len()
        ));
    }
    if tool > 0 {
        out.push(format!(
            "A rule a formatter, linter, compiler or measuring script already checks at {TOOL_CHECKED:.2}: {}.",
            count(tool, "line"),
        ));
    }
    if no_rule + tool > 0 {
        out.push("`--format json` shows each line with its answers.".into());
    }
    let unanswered = candidates.len() - answered.len();
    if unanswered > 0 {
        out.push(format!(
            "{} went unanswered: {}",
            count(unanswered, "line"),
            errors.join("; ")
        ));
    }
    out
}

fn usage_line(usage: &Usage) -> String {
    let cost = crate::output::cost(usage.usd);
    format!(
        "JevGate: {} API requests · {} input tokens{cost}",
        usage.requests, usage.tokens
    )
}

/// The new proposals as `[[question]]` tables for `jevgate.toml`.
pub fn toml(candidates: &[Candidate<'_>]) -> Result<String> {
    let tables: Result<Vec<String>> = with_status(candidates, Status::New)
        .iter()
        .filter_map(|c| c.proposal.as_ref())
        .map(super::proposal::Proposal::table)
        .collect();
    Ok(tables?.join("\n"))
}

/// Every candidate line with its answers and proposal.
pub fn json(
    files: &[File],
    skipped: &[Skipped],
    candidates: &[Candidate<'_>],
    (usage, errors): (&Usage, &[String]),
) -> Result<String> {
    let value = json!({
        "dry_run": false,
        "complete": errors.is_empty(),
        "errors": errors,
        "thresholds": {"rule": THRESHOLD, "tool": TOOL_CHECKED},
        "files": files_json(files),
        "skipped": skipped,
        "candidates": candidates.iter().map(candidate_json).collect::<Vec<_>>(),
        "api_requests": usage.requests,
        "paid_input_tokens": usage.tokens,
        "estimated_usd": usage.usd,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

fn files_json(files: &[File]) -> Vec<Value> {
    files
        .iter()
        .map(|f| json!({"path": slashed(&f.path), "lines": f.lines.len(), "paths": f.scope}))
        .collect()
}

fn line_json(file: &File, line: &super::lines::Line) -> Value {
    json!({
        "path": slashed(&file.path),
        "start_line": line.start_line,
        "end_line": line.end_line,
        "heading": line.heading,
        "lead_in": line.lead_in,
        "text": line.text,
    })
}

fn candidate_json(candidate: &Candidate<'_>) -> Value {
    let mut value = line_json(candidate.file, candidate.line);
    if let Some(answered) = &candidate.answered {
        value["convention"] = json!(answered.convention);
        value["unit"] = json!(answered.unit().0);
        value["units"] = json!(answered.units);
        value["checkers"] = json!(answered.checkers);
    }
    value["proposal"] = candidate
        .proposal
        .as_ref()
        .map_or(Value::Null, super::proposal::Proposal::describe);
    value
}
