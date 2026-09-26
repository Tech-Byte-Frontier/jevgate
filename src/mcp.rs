//! `jevgate mcp`: a Model Context Protocol server on stdin and stdout, so a
//! coding agent can run a check, read the last report's findings and look up
//! the rules as tools. Messages are newline-delimited JSON-RPC 2.0. A check
//! runs as a child process, so nothing it prints reaches the protocol stream.
use crate::{catalog, output, schema::Report};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

/// Protocol versions this server speaks; the first is offered when the
/// client asks for one it does not know.
const VERSIONS: [&str; 4] = ["2025-06-18", "2025-11-25", "2025-03-26", "2024-11-05"];
/// Findings returned by one `jevgate_findings` call.
const MAX_FINDINGS: usize = 50;

const INSTRUCTIONS: &str = "JevGate reviews code by asking TypeSafe Jev small questions about functions, files, tests and docs. \
Call jevgate_check with `base` (such as origin/main) to review what changed; it uses the repository's jevgate.toml and TYPESAFE_API_KEY, and paid requests only for code the answer cache lacks. \
Fix each `review` finding; for a `consider`, fix it or explain why the code should stay. Exit code 2 means the run could not finish: report it, never treat it as a pass. \
jevgate_findings reads the last report without running anything.";

pub fn run() -> Result<()> {
    let root = crate::config::repository_root(&std::env::current_dir()?.canonicalize()?);
    let server = Server {
        root,
        executable: std::env::current_exe().context("Cannot find the jevgate executable")?,
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle(&line) {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

struct Server {
    root: PathBuf,
    executable: PathBuf,
}

impl Server {
    /// The reply to one message, or none for a notification.
    fn handle(&self, line: &str) -> Option<Value> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Some(error(Value::Null, -32700, "Parse error"));
        };
        let id = message.get("id").cloned()?;
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        Some(match message["method"].as_str() {
            Some("initialize") => reply(id, initialize(&params)),
            Some("ping") => reply(id, json!({})),
            Some("tools/list") => reply(id, json!({"tools": tools()})),
            Some("tools/call") => reply(id, self.call(&params)),
            _ => error(id, -32601, "Method not found"),
        })
    }

    fn call(&self, params: &Value) -> Value {
        let arguments = &params["arguments"];
        let outcome = match params["name"].as_str() {
            Some("jevgate_check") => self.check(arguments),
            Some("jevgate_findings") => self.findings(arguments),
            Some("jevgate_rules") => {
                Ok(serde_json::to_string_pretty(&catalog::describe()).unwrap_or_default())
            }
            other => Err(anyhow::anyhow!("Unknown tool: {}", other.unwrap_or(""))),
        };
        match outcome {
            Ok(text) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
            Err(error) => {
                json!({"content": [{"type": "text", "text": format!("{error:#}")}], "isError": true})
            }
        }
    }

    /// Run `jevgate check` in the repository and return its agent output
    /// with the exit code's meaning.
    fn check(&self, arguments: &Value) -> Result<String> {
        let output = std::process::Command::new(&self.executable)
            .current_dir(&self.root)
            .args(check_arguments(arguments)?)
            .stdin(std::process::Stdio::null())
            .output()
            .context("Cannot start jevgate check")?;
        let mut text = String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string();
        let errors = String::from_utf8_lossy(&output.stderr);
        if !errors.trim().is_empty() {
            text.push_str(&format!("\n\n{}", errors.trim_end()));
        }
        let meaning = match output.status.code() {
            Some(0) if arguments["dry_run"].as_bool() == Some(true) => "dry run: nothing was sent",
            Some(0) => "exit 0: the gate passed",
            Some(1) => "exit 1: the gate failed; act on the findings",
            Some(2) => anyhow::bail!(
                "{}\n\n(exit 2: the run could not finish or the arguments are invalid; this is not a pass)",
                text.trim_start()
            ),
            _ => anyhow::bail!("{}\n\n(the check was interrupted)", text.trim_start()),
        };
        Ok(format!("{text}\n\n({meaning})"))
    }

    /// Findings of the last report, ranked, optionally for one path prefix.
    fn findings(&self, arguments: &Value) -> Result<String> {
        let report = crate::storage::read_latest(&self.root)
            .context("No report yet; call jevgate_check first")?;
        Ok(serde_json::to_string_pretty(&findings(
            &report,
            arguments["path"].as_str(),
            arguments["include_notes"].as_bool().unwrap_or(false),
        ))
        .unwrap_or_default())
    }
}

/// `check` arguments from a tool call. Values are passed as `--flag=value`
/// and paths after `--`, so no value is read as another flag.
fn check_arguments(arguments: &Value) -> Result<Vec<String>> {
    let mut args = vec![
        "check".to_string(),
        "--format=agent".into(),
        "--color=never".into(),
    ];
    if let Some(base) = arguments["base"].as_str() {
        args.push(format!("--base={base}"));
    }
    for rule in strings(&arguments["rules"], "rules")? {
        args.push(format!("--rule={rule}"));
    }
    for (flag, name) in [
        ("--include-tests", "include_tests"),
        ("--dry-run", "dry_run"),
        ("--verbose", "verbose"),
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

fn findings(report: &Report, prefix: Option<&str>, include_notes: bool) -> Value {
    let all: Vec<Value> = output::ranked(report)
        .into_iter()
        .filter(|(path, _)| prefix.is_none_or(|p| path.starts_with(p)))
        .filter(|(_, f)| include_notes || f.strength != crate::schema::Strength::Note)
        .map(|(path, f)| {
            json!({
                "path": path,
                "line": f.line,
                "rule": f.rule,
                "strength": output::label(&f.strength),
                "message": f.message,
                "action": f.action,
                "probability": f.concern_probability,
                "baselined": f.baselined,
                "suppressed": f.suppressed,
            })
        })
        .collect();
    json!({
        "headline": output::headline(report),
        "complete": report.complete,
        "gate": report.gate,
        "shown": all.len().min(MAX_FINDINGS),
        "total": all.len(),
        "findings": all.into_iter().take(MAX_FINDINGS).collect::<Vec<_>>(),
    })
}

fn initialize(params: &Value) -> Value {
    let asked = params["protocolVersion"].as_str().unwrap_or_default();
    let version = VERSIONS
        .iter()
        .find(|v| **v == asked)
        .unwrap_or(&VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "jevgate", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "jevgate_check",
            "title": "Review code with JevGate",
            "description": "Run `jevgate check` in the repository and return its ranked findings, each with a location, probability and next step. Uses jevgate.toml and TYPESAFE_API_KEY; unchanged code is answered from the cache for free, and dry_run costs nothing. Can take minutes on a large change.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "base": {"type": "string", "description": "Review only files changed since this Git revision, such as origin/main"},
                    "paths": {"type": "array", "items": {"type": "string"}, "description": "Files or directories to review instead of the discovered source"},
                    "rules": {"type": "array", "items": {"type": "string"}, "description": "Rule IDs, keys or groups, such as security; replaces the configured selection"},
                    "include_tests": {"type": "boolean", "description": "Also judge tests"},
                    "dry_run": {"type": "boolean", "description": "List the files and planned requests without sending anything"},
                    "verbose": {"type": "boolean", "description": "Also show optional notes and per-file detail"},
                },
                "additionalProperties": false,
            },
        },
        {
            "name": "jevgate_findings",
            "title": "Read the last JevGate report",
            "description": "Return the findings of the last check in this repository (.jevgate/latest.json), ranked, without running anything.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Only findings in files under this path"},
                    "include_notes": {"type": "boolean", "description": "Also return optional notes"},
                },
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "jevgate_rules",
            "title": "List JevGate's rules",
            "description": "Every rule with its ID, group, default, the question it asks and what it looks at.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true},
        },
    ])
}

fn reply(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn server() -> Server {
        Server {
            root: Path::new(".").into(),
            executable: PathBuf::from("jevgate"),
        }
    }

    #[test]
    fn initialize_lists_tools_and_answers_unknown_methods() {
        let server = server();
        let init = server
            .handle(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#)
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "jevgate");
        let newer = server
            .handle(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#)
            .unwrap();
        assert_eq!(newer["result"]["protocolVersion"], VERSIONS[0]);
        assert!(
            server
                .handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );
        let list = server
            .handle(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#)
            .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["jevgate_check", "jevgate_findings", "jevgate_rules"]
        );
        let unknown = server
            .handle(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#)
            .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        assert_eq!(server.handle("not json").unwrap()["error"]["code"], -32700);
    }

    #[test]
    fn tool_arguments_never_become_flags() {
        let args = check_arguments(&json!({
            "base": "--config=/etc/passwd",
            "rules": ["security"],
            "paths": ["--refresh", "src"],
            "dry_run": true,
        }))
        .unwrap();
        assert_eq!(
            args,
            [
                "check",
                "--format=agent",
                "--color=never",
                "--base=--config=/etc/passwd",
                "--rule=security",
                "--dry-run",
                "--",
                "--refresh",
                "src"
            ]
        );
        assert!(check_arguments(&json!({"paths": "src"})).is_err());
    }

    #[test]
    fn a_failed_tool_call_is_a_tool_error_not_a_protocol_error() {
        let reply = server()
            .handle(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#)
            .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        let rules = server()
            .handle(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"jevgate_rules"}}"#)
            .unwrap();
        assert_eq!(rules["result"]["isError"], false);
        assert!(
            rules["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("security/injection")
        );
    }
}
