//! The MCP server over stdin and stdout, driven by a scripted client.
use super::*;
use mock_provider::{MockProvider, Reply, answer};
use serde_json::{Value, json};
use std::io::Write;

const APP: &str = "def total(rows):\n    s = 0\n    for r in rows:\n        s += r\n    s = s * 2\n    return s + 1\n";

/// Every message `jevgate mcp`, started by `command`, sends while
/// answering `requests`, in order.
fn session(mut command: Command, requests: &[Value]) -> Vec<Value> {
    let mut child = command
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for request in requests {
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

fn call(id: u64, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
}

/// A `jevgate_check` call that asks for progress with `token`.
fn check_with_progress(id: u64, token: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": "jevgate_check", "arguments": {}, "_meta": {"progressToken": token}}})
}

/// The reply to request `id`, and the notifications sent after the reply
/// before it.
fn reply_to(messages: &[Value], id: u64) -> (&Value, Vec<&Value>) {
    let at = messages.iter().position(|m| m["id"] == id).unwrap();
    let mut before: Vec<&Value> = messages[..at]
        .iter()
        .rev()
        .take_while(|m| m.get("id").is_none())
        .collect();
    before.reverse();
    (&messages[at], before)
}

/// Write a clear answer to every first-pass request a dry run plans into
/// the project's answer cache, keyed and stored as a run saves it
/// (`requests::judgment_key`, `storage::Store::save`), so a check is
/// answered wholly from the cache and needs no key. Returns how many.
fn cache_clear_answers(project: &Project) -> usize {
    use sha2::Digest;
    let preview = project.preview(&["check", "--dry-run", "--show-requests", "--format", "json"]);
    let requests = preview["initial_requests"].as_array().unwrap();
    let cache = project.0.join(".jevgate/cache");
    std::fs::create_dir_all(&cache).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for request in requests {
        let answers: serde_json::Map<String, Value> = request["questions"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, question)| (key.clone(), clear(question)))
            .collect();
        let identity = serde_json::to_vec(&("jevgate-units-v1", request)).unwrap();
        let hash: String = sha2::Sha256::digest(identity)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let entry = json!({"request_hash": hash, "created_at": now, "response": {
            "model": request["model"], "answers": answers,
            "usage": {"input_tokens": 10, "output_tokens": 0}}});
        std::fs::write(cache.join(format!("{hash}.json")), entry.to_string()).unwrap();
    }
    requests.len()
}

/// A certain answer at the bottom of the question's scale: nothing to report.
fn clear(question: &Value) -> Value {
    let certain = |options: Vec<String>, chosen: &str| -> Value {
        options
            .iter()
            .map(|option| {
                let p = if option == chosen { 1.0 } else { 0.0 };
                (option.clone(), json!(p))
            })
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    match question["type"].as_str().unwrap() {
        "noul" => json!({"type": "noul", "noul": 0.02}),
        "score" => {
            let levels = question["criteria"].as_array().unwrap().len();
            let options = (0..levels).map(|level| level.to_string()).collect();
            json!({"type": "score", "score": 0.0, "confidence": 1.0,
                "probabilities": certain(options, "0")})
        }
        _ => {
            let options: Vec<String> = question["criteria"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            let chosen = if options.iter().any(|o| o == "none") {
                "none".to_string()
            } else {
                options[0].clone()
            };
            json!({"type": "choice", "choice": chosen, "confidence": 1.0,
                "probabilities": certain(options, &chosen)})
        }
    }
}

#[test]
fn mcp_returns_structured_results_and_reports_a_checks_progress() {
    let project = Project::new();
    std::fs::write(project.0.join("app.py"), APP).unwrap();
    let cached = cache_clear_answers(&project);
    assert!(cached > 0);
    let messages = session(
        project.command(),
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            call(3, "jevgate_check", json!({"dry_run": true})),
            check_with_progress(4, json!("p4")),
            call(5, "jevgate_findings", json!({"max_verify": 0})),
            call(6, "jevgate_findings", json!({"max_findings": 0})),
            call(7, "no_such_tool", json!({})),
        ],
    );
    let (init, _) = reply_to(&messages, 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    let (list, _) = reply_to(&messages, 2);
    for tool in list["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["outputSchema"]["type"], "object", "{}", tool["name"]);
    }
    let (dry, _) = reply_to(&messages, 3);
    let dry = &dry["result"];
    assert_eq!(dry["isError"], false);
    assert_eq!(dry["structuredContent"]["dry_run"], true);
    assert_eq!(dry["structuredContent"]["planned"]["requests"], cached);
    assert_eq!(
        dry["structuredContent"]["planned"]["cached"], cached,
        "every request is answered by the cache"
    );
    let planned = &dry["structuredContent"]["planned"];
    assert!(planned["questions"].as_u64().unwrap() >= cached as u64);
    assert_eq!(
        planned["cached_questions"], planned["questions"],
        "and so is every question"
    );
    let text = dry["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("JevGate: dry run · 1 files"), "{text}");
    assert!(text.ends_with("(dry run: nothing was sent)"));
    let (check, progress) = reply_to(&messages, 4);
    let result = &check["result"];
    assert_eq!(result["isError"], false, "{result}");
    let structured = &result["structuredContent"];
    assert_eq!(structured["complete"], true);
    assert_eq!(structured["exit_code"], 0);
    assert_eq!(structured["gate"]["passed"], true);
    assert_eq!(structured["usage"]["api_requests"], 0, "nothing was sent");
    assert_eq!(structured["findings"], json!([]));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("JevGate: clear · gate passed"), "{text}");
    assert!(text.ends_with("(exit 0: the gate passed)"));
    // Progress before the reply: the start, then the answered first pass.
    let steps: Vec<(&Value, &Value)> = progress
        .iter()
        .map(|n| {
            assert_eq!(n["method"], "notifications/progress");
            (&n["params"]["progressToken"], &n["params"]["progress"])
        })
        .collect();
    assert_eq!(
        steps,
        [(&json!("p4"), &json!(0)), (&json!("p4"), &json!(cached))]
    );
    let (findings, _) = reply_to(&messages, 5);
    let last = &findings["result"]["structuredContent"];
    assert_eq!(last["headline"], structured["headline"], "the same report");
    assert_eq!(
        findings["result"]["content"][0]["text"].as_str().unwrap(),
        last.to_string(),
        "the text is the structured result as JSON"
    );
    let (refused, _) = reply_to(&messages, 6);
    assert_eq!(refused["result"]["isError"], true);
    assert!(
        refused["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("`max_findings` must be a whole number from 1 to 200")
    );
    let (unknown, _) = reply_to(&messages, 7);
    assert_eq!(unknown["error"]["code"], -32602);
}

#[test]
fn mcp_reports_an_incomplete_check_as_an_error_that_says_why() {
    let project = Project::new();
    std::fs::write(project.0.join("app.py"), APP).unwrap();
    let messages = session(
        project.command(),
        &[
            check_with_progress(1, json!(7)),
            call(2, "jevgate_check", json!({"base": "no-such-revision"})),
        ],
    );
    let (check, progress) = reply_to(&messages, 1);
    assert_eq!(progress.len(), 1, "the start; no request was answered");
    assert_eq!(progress[0]["params"]["progressToken"], 7);
    let result = &check["result"];
    assert_eq!(result["isError"], true, "exit 2 is never a pass");
    let structured = &result["structuredContent"];
    assert_eq!(structured["complete"], false);
    assert_eq!(structured["exit_code"], 2);
    assert!(
        structured["errors"][0]
            .as_str()
            .unwrap()
            .starts_with("Failed 1: No API key configured"),
        "{structured}"
    );
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\nFailed 1: No API key configured"), "{text}");
    assert!(text.ends_with("this is not a pass)"), "{text}");
    let (base, _) = reply_to(&messages, 2);
    assert_eq!(base["result"]["isError"], true);
    assert!(
        base["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no-such-revision")
    );
}

#[test]
fn mcp_lists_the_repositorys_custom_questions_beside_the_rules() {
    let project = Project::new();
    std::fs::create_dir_all(project.0.join(".jevgate/questions")).unwrap();
    std::fs::write(
        project.0.join(".jevgate/questions/no-body-logs.toml"),
        "question = \"Does this function write a request body to a log?\"\nunit = \"function\"\n",
    )
    .unwrap();
    let messages = session(project.command(), &[call(1, "jevgate_rules", json!({}))]);
    let (reply, _) = reply_to(&messages, 1);
    let result = &reply["result"];
    assert_eq!(result["isError"], false, "{result}");
    let listed = result["structuredContent"]["rules"].as_array().unwrap();
    let ids: Vec<&str> = listed
        .iter()
        .map(|rule| rule["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"security/injection"), "{ids:?}");
    assert_eq!(
        ids.last(),
        Some(&"custom/no-body-logs"),
        "the repository's questions too"
    );
    let source = listed.last().unwrap()["custom"]["source"].as_str().unwrap();
    assert!(source.ends_with("no-body-logs.toml"), "{source}");
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        text, result["structuredContent"],
        "the text is the same JSON"
    );
}

#[test]
fn a_run_stopped_by_exhausted_credits_says_why_in_the_agent_text_and_to_the_agent() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    let provider = MockProvider::start(|_| {
        Reply::json(402, &json!({"error": "private"}))
            .header("x-typesafe-request-id", "req_mock402")
    });
    let run = || project.asking(&provider, "key");
    let reason = "Failed 1: TypeSafe HTTP 402 (credits exhausted; add credits or turn on auto-refill at https://console.typesafe.ai); request was not retried; request id req_mock402";
    let output = run().args(["check", "."]).output().unwrap();
    let agent = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(2), "{agent}");
    assert!(agent.contains("\n1 file failed.\n"), "{agent}");
    assert!(agent.contains(reason), "{agent}");
    assert!(!agent.contains("private"), "{agent}");
    let messages = session(run(), &[call(1, "jevgate_check", json!({}))]);
    let (reply, _) = reply_to(&messages, 1);
    let result = &reply["result"];
    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(reason),
        "{result}"
    );
    assert_eq!(
        result["structuredContent"]["errors"],
        json!([reason]),
        "the structured result says why too"
    );
}

#[test]
fn findings_say_how_the_checks_gate_counted_them() {
    // Every answer at the top of its scale: the long function is a
    // function-simplification review, which the default gate fails on, and
    // the region a hardcoded-values review, which it still measures.
    let project = Project::new();
    std::fs::write(
        project.0.join("jevgate.toml"),
        "rules = [\"hardcoded-values\", \"function-simplification\"]\n",
    )
    .unwrap();
    let region = format!("const REGION: &str = \"eu-west-1\";\n{LONG_RS}");
    std::fs::write(project.0.join("lib.rs"), region).unwrap();
    let provider = MockProvider::start(|received| Reply::json(200, &answer(&received.json(), 2)));
    let messages = session(
        project.asking(&provider, "key"),
        &[
            call(1, "jevgate_check", json!({})),
            call(2, "jevgate_findings", json!({})),
        ],
    );
    for id in [1, 2] {
        let (reply, _) = reply_to(&messages, id);
        let result = &reply["result"]["structuredContent"];
        let counted: Vec<(&str, &str)> = result["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["rule"].as_str().unwrap(), f["gate"].as_str().unwrap()))
            .collect();
        assert_eq!(
            counted,
            [
                ("maintainability/function-simplification", "fails"),
                ("maintainability/hardcoded-values", "measuring")
            ],
            "{id}: failures first, as the check's gate counted them"
        );
        assert_eq!(result["exit_code"], 1, "{id}");
        assert_eq!(result["gate"]["reasons"], json!(["1 new review finding"]));
    }
}
