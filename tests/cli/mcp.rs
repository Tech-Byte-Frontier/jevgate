//! The MCP server over stdin and stdout.
use super::*;
use std::io::Write;

#[test]
fn mcp_answers_each_request_on_its_own_line_and_runs_a_dry_check() {
    let project = Project::new();
    std::fs::write(
        project.0.join("app.py"),
        "def total(rows):\n    s = 0\n    for r in rows:\n        s += r\n    s = s * 2\n    return s + 1\n",
    )
    .unwrap();
    let mut child = project
        .command()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"jevgate_check","arguments":{"dry_run":true}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"jevgate_check","arguments":{"base":"no-such-revision"}}}"#,
    ];
    let mut stdin = child.stdin.take().unwrap();
    for request in requests {
        writeln!(stdin, "{request}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let replies: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 3, "the notification gets no reply");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    let dry = &replies[1]["result"];
    assert_eq!(dry["isError"], false);
    let text = dry["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("JevGate: dry run · 1 files"), "{text}");
    assert!(text.ends_with("(dry run: nothing was sent)"));
    let failed = &replies[2]["result"];
    assert_eq!(failed["isError"], true, "exit 2 is never a pass");
    assert!(
        failed["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no-such-revision")
    );
    assert!(!project.0.join(".jevgate/latest.json").exists());
}
