//! The MCP server over stdin and stdout.
use super::*;
use mock_provider::{MockProvider, Reply};
use std::io::Write;

/// The requests that open a session: `initialize`, then the notification
/// that ends it, which gets no reply.
const OPENING: [&str; 2] = [
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
];

/// The replies of `jevgate mcp`, started by `command`, to the opening
/// requests and then `calls`, one reply per line.
fn replies(mut command: Command, calls: &[&str]) -> Vec<serde_json::Value> {
    let mut child = command
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for request in OPENING.iter().chain(calls) {
        writeln!(stdin, "{request}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// A tool call's text.
fn text(result: &serde_json::Value) -> &str {
    result["content"][0]["text"].as_str().unwrap()
}

#[test]
fn mcp_answers_each_request_on_its_own_line_and_runs_a_dry_check() {
    let project = Project::new();
    std::fs::write(
        project.0.join("app.py"),
        "def total(rows):\n    s = 0\n    for r in rows:\n        s += r\n    s = s * 2\n    return s + 1\n",
    )
    .unwrap();
    let replies = replies(
        project.command(),
        &[
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"jevgate_check","arguments":{"dry_run":true}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"jevgate_check","arguments":{"base":"no-such-revision"}}}"#,
        ],
    );
    assert_eq!(replies.len(), 3, "the notification gets no reply");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    let dry = &replies[1]["result"];
    assert_eq!(dry["isError"], false);
    assert!(text(dry).starts_with("JevGate: dry run · 1 files"), "{dry}");
    assert!(text(dry).ends_with("(dry run: nothing was sent)"));
    let failed = &replies[2]["result"];
    assert_eq!(failed["isError"], true, "exit 2 is never a pass");
    assert!(text(failed).contains("no-such-revision"));
    assert!(!project.0.join(".jevgate/latest.json").exists());
}

#[test]
fn a_run_stopped_by_exhausted_credits_says_why_in_the_agent_text_and_to_the_agent() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    let provider = MockProvider::start(|_| {
        Reply::json(402, &serde_json::json!({"error": "private"}))
            .header("x-typesafe-request-id", "req_mock402")
    });
    let run = || {
        let mut command = project.command();
        command
            .env("TYPESAFE_API_KEY", "key")
            .env("JEVGATE_BASE_URL", &provider.url);
        command
    };
    let reason = "Failed 1: TypeSafe HTTP 402 (credits exhausted; add credits or turn on auto-refill at https://console.typesafe.ai); request was not retried; request id req_mock402";
    let output = run().args(["check", "."]).output().unwrap();
    let agent = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(2), "{agent}");
    assert!(agent.contains("\n1 file failed.\n"), "{agent}");
    assert!(agent.contains(reason), "{agent}");
    assert!(!agent.contains("private"), "{agent}");
    let replies = replies(
        run(),
        &[
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"jevgate_check","arguments":{}}}"#,
        ],
    );
    let result = &replies[1]["result"];
    assert_eq!(result["isError"], true);
    assert!(text(result).contains(reason), "{result}");
}
