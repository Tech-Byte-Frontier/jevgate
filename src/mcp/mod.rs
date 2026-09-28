//! `jevgate mcp`: a Model Context Protocol server on stdin and stdout, so a
//! coding agent can run a check, read the last report's findings and look up
//! the rules as tools. Messages are newline-delimited JSON-RPC 2.0. A check
//! runs as a child process, so nothing it prints reaches the protocol stream.
//! `tools` declares the tools and their schemas, `results` shapes a report
//! for an agent, and `checking` runs a check and reports its progress.
mod checking;
mod results;
mod tools;

use crate::{catalog, schema::Report};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

/// Protocol versions this server speaks; the first is offered when the
/// client asks for one it does not know.
const VERSIONS: [&str; 4] = ["2025-06-18", "2025-11-25", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "JevGate reviews code by asking TypeSafe Jev small questions about functions, files, tests and docs. \
Call jevgate_check with `base` (such as origin/main) to review what changed; it uses the repository's jevgate.toml and the API key `jevgate auth status` shows, and paid requests only for code the answer cache lacks. \
Fix each finding marked to fail the gate (`gate: fails`): they decide the exit code, and by default only rules and levels measured right at least 80% of the time on projects JevGate was never tuned on fail it. \
Weigh the other `review` and `consider` findings: fix one when it is right, or say why the code should stay. \
Each `verify` item is a question Jev left undecided about a unit, with the evidence it named and the probability of each answer: read the code there and change it only if you agree it should change; verify items never fail the gate. \
Each `guards` entry is something the change does to the checks around the code, such as a suppression, a skipped test or an edit to jevgate.toml: tell the person, who decides; guards never fail the gate. \
Exit code 2 means the run could not finish: report it, never treat it as a pass. \
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
        // A client that stops reading fails the reply's write below.
        let reply = server.handle(&line, &mut |notification| {
            let _ = send(&mut stdout, &notification);
        });
        if let Some(reply) = reply {
            send(&mut stdout, &reply)?;
        }
    }
    Ok(())
}

fn send(out: &mut impl Write, message: &Value) -> std::io::Result<()> {
    writeln!(out, "{message}")?;
    out.flush()
}

struct Server {
    root: PathBuf,
    executable: PathBuf,
}

impl Server {
    /// The reply to one message, or none for a notification. `notify` sends
    /// the notifications that come before the reply, such as a check's
    /// progress.
    fn handle(&self, line: &str, notify: &mut dyn FnMut(Value)) -> Option<Value> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Some(error(Value::Null, -32700, "Parse error"));
        };
        let id = message.get("id").cloned()?;
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        Some(match message["method"].as_str() {
            Some("initialize") => reply(id, initialize(&params)),
            Some("ping") => reply(id, json!({})),
            Some("tools/list") => reply(id, json!({"tools": tools::list()})),
            Some("tools/call") => match self.call(&params, notify) {
                Some(result) => reply(id, result.into_value()),
                None => error(
                    id,
                    -32602,
                    &format!("Unknown tool: {}", params["name"].as_str().unwrap_or("")),
                ),
            },
            _ => error(id, -32601, "Method not found"),
        })
    }

    /// A tool's result, or none for a tool this server does not have: an
    /// unknown tool is a protocol error, while a tool that fails, invalid
    /// arguments included, answers with a result marked as an error.
    fn call(&self, params: &Value, notify: &mut dyn FnMut(Value)) -> Option<Outcome> {
        let arguments = &params["arguments"];
        let outcome = match params["name"].as_str()? {
            "jevgate_check" => {
                let token = params["_meta"]["progressToken"].clone();
                self.check(arguments, (!token.is_null()).then_some(token), notify)
            }
            "jevgate_findings" => self.findings(arguments),
            "jevgate_rules" => Ok(Outcome::structured(json!({"rules": catalog::describe()}))),
            _ => return None,
        };
        Some(outcome.unwrap_or_else(|error| Outcome::failed(format!("{error:#}"))))
    }

    /// Run `jevgate check` in the repository, sending its progress when the
    /// call asked for it with a token.
    fn check(
        &self,
        arguments: &Value,
        token: Option<Value>,
        notify: &mut dyn FnMut(Value),
    ) -> Result<Outcome> {
        let verbose = arguments["verbose"].as_bool() == Some(true);
        let selection = results::Selection::new(arguments, None, verbose)?;
        let mut command = std::process::Command::new(&self.executable);
        command
            .current_dir(&self.root)
            .args(checking::arguments(arguments)?);
        let mut progress = token.map(checking::Progress::new);
        let finished = checking::run(&mut command, &mut |report| {
            if let Some(notification) = progress.as_mut().and_then(|p| p.notification(report)) {
                notify(notification);
            }
        })?;
        Ok(finished.outcome(&selection, verbose))
    }

    /// Findings and verify items of the last report, ranked, optionally for
    /// one path prefix.
    fn findings(&self, arguments: &Value) -> Result<Outcome> {
        let notes = arguments["include_notes"].as_bool() == Some(true);
        let selection = results::Selection::new(arguments, arguments["path"].as_str(), notes)?;
        let report: Report = crate::storage::read_latest(&self.root)
            .context("No report yet; call jevgate_check first")?;
        let exit_code = results::exit_code(&report);
        Ok(Outcome::structured(results::structured(
            &report, &selection, exit_code,
        )))
    }
}

/// A tool's result: the text older clients read, and the structured result
/// that tools declaring an output schema return.
struct Outcome {
    text: String,
    structured: Option<Value>,
    error: bool,
}

impl Outcome {
    /// A structured result whose text is the same result as compact JSON.
    fn structured(structured: impl serde::Serialize) -> Self {
        let structured = serde_json::to_value(structured).unwrap_or_default();
        Self {
            text: structured.to_string(),
            structured: Some(structured),
            error: false,
        }
    }

    /// A tool that could not do what it was asked, and why.
    fn failed(text: String) -> Self {
        Self {
            text,
            structured: None,
            error: true,
        }
    }

    fn into_value(self) -> Value {
        let mut result = json!({
            "content": [{"type": "text", "text": self.text}],
            "isError": self.error,
        });
        if let Some(structured) = self.structured {
            result["structuredContent"] = structured;
        }
        result
    }
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

    fn server(root: &Path) -> Server {
        Server {
            root: root.into(),
            executable: PathBuf::from("jevgate"),
        }
    }

    /// The reply to one message; no call here sends progress.
    fn exchange(server: &Server, line: &str) -> Option<Value> {
        server.handle(line, &mut |_| {})
    }

    #[test]
    fn initialize_lists_tools_and_answers_unknown_methods() {
        let server = server(Path::new("."));
        let ask = |line: &str| exchange(&server, line);
        let init = ask(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "jevgate");
        let newer = ask(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#).unwrap();
        assert_eq!(newer["result"]["protocolVersion"], VERSIONS[0]);
        assert!(ask(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        let list = ask(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#).unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            ["jevgate_check", "jevgate_findings", "jevgate_rules"]
        );
        assert!(
            tools.iter().all(|t| t["outputSchema"]["type"] == "object"),
            "every tool declares its structured result"
        );
        let unknown = ask(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        assert_eq!(ask("not json").unwrap()["error"]["code"], -32700);
    }

    #[test]
    fn an_unknown_tool_is_a_protocol_error_and_a_failed_tool_a_tool_error() {
        let project = crate::tests::Project::new();
        let server = server(&project.0);
        let unknown = exchange(
            &server,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#,
        );
        let unknown = unknown.unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
        assert_eq!(unknown["error"]["message"], "Unknown tool: nope");
        let missing = exchange(
            &server,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"jevgate_findings"}}"#,
        );
        let missing = &missing.unwrap()["result"];
        assert_eq!(missing["isError"], true);
        assert!(missing.get("structuredContent").is_none());
        assert!(
            missing["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("No report yet")
        );
    }

    #[test]
    fn rules_come_back_structured_and_as_the_same_json_text() {
        let rules = exchange(
            &server(Path::new(".")),
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"jevgate_rules"}}"#,
        );
        let result = &rules.unwrap()["result"];
        assert_eq!(result["isError"], false);
        let structured = &result["structuredContent"];
        assert!(
            structured["rules"]
                .as_array()
                .unwrap()
                .iter()
                .any(|rule| rule["id"] == "security/injection")
        );
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(&text, structured);
        tools::assert_conforms(structured, &tools::list()[2]["outputSchema"]);
    }
}
