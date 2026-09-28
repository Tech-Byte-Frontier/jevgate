//! Running `jevgate check` for a tool call: its arguments, its report
//! snapshots read as the child publishes them (`--format jsonl` prints one
//! at the start and after each stage), and the tool result a finished check
//! becomes. The last snapshot is the report, so the text and the structured
//! result come from the same one.
use super::{
    Outcome,
    results::{Selection, structured},
};
use crate::{output, schema::Report};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
};

/// `check` arguments from a tool call. Values are passed as `--flag=value`
/// and paths after `--`, so no value is read as another flag.
pub(super) fn arguments(arguments: &Value) -> Result<Vec<String>> {
    let mut args = vec!["check".to_string(), "--format=jsonl".into()];
    if let Some(base) = arguments["base"].as_str() {
        args.push(format!("--base={base}"));
    }
    for rule in strings(&arguments["rules"], "rules")? {
        args.push(format!("--rule={rule}"));
    }
    for (flag, name) in [
        ("--whole-files", "whole_files"),
        ("--include-tests", "include_tests"),
        ("--dry-run", "dry_run"),
    ] {
        if arguments[name].as_bool() == Some(true) {
            args.push(flag.into());
        }
    }
    let paths = strings(&arguments["paths"], "paths")?;
    if !paths.is_empty() {
        args.push("--".into());
        args.extend(paths);
    }
    Ok(args)
}

fn strings(value: &Value, name: &str) -> Result<Vec<String>> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .with_context(|| format!("`{name}` must be a list of strings"))
            })
            .collect(),
        _ => anyhow::bail!("`{name}` must be a list of strings"),
    }
}

/// A check that ran to its end or stopped: its last report, what it wrote
/// to stderr and its exit code.
pub(super) struct Finished {
    report: Option<Report>,
    stderr: String,
    code: Option<i32>,
}

/// Run the check to its end.
pub(super) fn run(command: &mut Command) -> Result<Finished> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot start jevgate check")?;
    let mut stderr = child
        .stderr
        .take()
        .context("No stderr from jevgate check")?;
    // Read apart, so a child that fills the stderr pipe never waits on it.
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let stdout = child
        .stdout
        .take()
        .context("No stdout from jevgate check")?;
    let report = last_snapshot(BufReader::new(stdout));
    let status = child.wait().context("jevgate check did not finish")?;
    Ok(Finished {
        report,
        stderr: errors.join().unwrap_or_default(),
        code: status.code(),
    })
}

/// The last report of a stream of snapshots, one per line. A line that is
/// not a report counts as none: the last line decides. The stream is
/// dropped on return, so a child still writing after a read error gets a
/// closed pipe instead of waiting.
fn last_snapshot(mut lines: impl BufRead) -> Option<Report> {
    let mut line = Vec::new();
    let mut report = None;
    while lines.read_until(b'\n', &mut line).unwrap_or(0) > 0 {
        report = serde_json::from_slice::<Report>(&line).ok();
        line.clear();
    }
    report
}

impl Finished {
    /// The tool result: the agent text, the verify items, what the child
    /// wrote to stderr and what its exit code means; with the structured
    /// result when it published a report. Any exit but 0 and 1 is an error:
    /// an incomplete run is never a pass.
    pub(super) fn outcome(self, selection: &Selection, verbose: bool) -> Outcome {
        let dry_run = self.report.as_ref().is_some_and(|report| report.dry_run);
        let meaning = match self.code {
            Some(0) if dry_run => "dry run: nothing was sent",
            Some(0) => "exit 0: the gate passed",
            Some(1) => "exit 1: the gate failed; act on the findings",
            Some(2) => {
                "exit 2: the run could not finish or the arguments are invalid; this is not a pass"
            }
            _ => "the check was interrupted",
        };
        let mut sections = Vec::new();
        let mut structured_result = None;
        if let Some(report) = &self.report {
            let exit_code = self
                .code
                .and_then(|code| u8::try_from(code).ok())
                .unwrap_or_else(|| super::results::exit_code(report));
            let result = structured(report, selection, exit_code);
            let mut text = Vec::new();
            let _ = output::agent(&mut text, report, verbose, output::Style::PLAIN);
            sections.push(String::from_utf8_lossy(&text).trim_end().to_string());
            sections.extend(result.verify_text());
            structured_result = serde_json::to_value(result).ok();
        }
        if !self.stderr.trim().is_empty() {
            sections.push(self.stderr.trim_end().to_string());
        }
        sections.push(format!("({meaning})"));
        Outcome {
            text: sections.join("\n\n"),
            structured: structured_result,
            error: !matches!(self.code, Some(0 | 1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_arguments_never_become_flags() {
        let args = arguments(&json!({
            "base": "--config=/etc/passwd",
            "rules": ["security"],
            "paths": ["--refresh", "src"],
            "whole_files": true,
            "dry_run": true,
        }))
        .unwrap();
        assert_eq!(
            args,
            [
                "check",
                "--format=jsonl",
                "--base=--config=/etc/passwd",
                "--rule=security",
                "--whole-files",
                "--dry-run",
                "--",
                "--refresh",
                "src"
            ]
        );
        assert!(arguments(&json!({"paths": "src"})).is_err());
    }

    #[test]
    fn an_incomplete_check_is_an_error_whose_structured_result_says_why() {
        let project = crate::tests::Project::new();
        project.write("a.rs", &crate::tests::function("a"));
        let mut mock = crate::tests::Mock {
            malformed: true,
            ..Default::default()
        };
        let report = crate::tests::run(&project, &crate::tests::args(), &mut mock);
        let finished = Finished {
            report: Some(report),
            stderr: "jevgate: 1 files failed".into(),
            code: Some(2),
        };
        let selection = Selection::new(&json!({}), None, false).unwrap();
        let outcome = finished.outcome(&selection, false);
        assert!(outcome.error);
        let structured = outcome.structured.unwrap();
        assert_eq!(structured["complete"], false);
        assert_eq!(structured["exit_code"], 2);
        assert!(
            structured["errors"][0]
                .as_str()
                .unwrap()
                .starts_with("Failed 1: "),
            "{structured}"
        );
        assert!(outcome.text.contains("\nFailed 1: "), "{}", outcome.text);
        assert!(
            outcome.text.ends_with(
                "jevgate: 1 files failed\n\n(exit 2: the run could not finish or the arguments are invalid; this is not a pass)"
            ),
            "{}",
            outcome.text
        );
    }

    #[test]
    fn a_check_that_printed_no_report_is_an_error_with_its_stderr() {
        let finished = Finished {
            report: None,
            stderr: "jevgate: Unknown revision no-such-revision\n".into(),
            code: Some(2),
        };
        let selection = Selection::new(&json!({}), None, false).unwrap();
        let outcome = finished.outcome(&selection, false);
        assert!(outcome.error && outcome.structured.is_none());
        assert!(outcome.text.starts_with("jevgate: Unknown revision"));
    }
}
