//! A team's convention through the whole cycle, as 0.29's done-when asks:
//! proposed from its AGENTS.md line, edited and accepted by a person, its
//! examples passing `rules test`, and a pull request check failing on a
//! change that breaks it and passing once the change is fixed. The Stop
//! hook's side of the done-when needs 0.27's `jevgate hook`.
use super::*;
use mock_provider::{MockProvider, Reply, answer};
use serde_json::{Value, json};

const AGENTS: &str =
    "# Conventions\n\n- Never log request bodies.\n- Run `cargo test` before pushing.\n";

/// What a person adds to the proposal before accepting it, as the proposal
/// says: a line of guidance, and a failing and a passing example from the
/// code; with the level raised, it fails the gate.
const GUIDANCE: &str = "guidance = \"Logging a request's id, method or path is fine; logging its body, or a field of it, breaks the rule.\"\n";

const EXAMPLES: &str = r#"
[[failing]]
path = "src/orders.rs"
code = '''
fn charge(request: &Request) {
    log(&request.body);
}
'''

[[passing]]
path = "src/orders.rs"
code = '''
fn charge(request: &Request) {
    log(&request.id);
}
'''
"#;

/// A change that breaks the rule: a function logging a request's body.
const BREAKING: &str = "fn charge(request: &Request) -> u32 {\n    log(&request.body);\n    let total = request.total();\n    let tax = total / 10;\n    let fee = 1;\n    total + tax + fee\n}\n";

/// Jev as the test scripts it: a line is a rule when it says "Never", a
/// function shows it and a reviewer checks it; a function breaks the rule
/// when it logs a request's body; every other question is clear.
fn scripted() -> MockProvider {
    MockProvider::start(|received| {
        let request = received.json();
        let mut body = answer(&request, 0);
        for (key, slot) in body["answers"].as_object_mut().unwrap() {
            if let Some(scripted) = scripted_answer(&request, key) {
                *slot = scripted;
            }
        }
        Reply::json(200, &body)
    })
}

/// The scripted answer to the question `key` of `request`, when the test
/// scripts it: a custom question about a function, or a proposal question
/// about a candidate line.
fn scripted_answer(request: &Value, key: &str) -> Option<Value> {
    if let Some(rest) = key.strip_prefix("custom_") {
        let index: usize = rest.split('_').next()?.parse().ok()?;
        let source = request["state"]["functions"][index]["source"].as_str()?;
        let logs = source.contains("log(") && source.contains(".body");
        return Some(json!({"type": "noul", "noul": if logs { 0.93 } else { 0.04 }}));
    }
    let (position, question) = key.strip_prefix('c')?.split_once('_')?;
    let line = &request["state"]["candidates"][position.parse::<usize>().ok()?];
    let rule = line["text"].as_str()?.starts_with("Never");
    Some(match question {
        "convention" => json!({"type": "noul", "noul": if rule { 0.95 } else { 0.05 }}),
        "unit" => chosen(&request["questions"][key], "function"),
        _ => chosen(&request["questions"][key], "reviewer"),
    })
}

/// A Choice of `option` at 0.9 among the question's options.
fn chosen(question: &Value, option: &str) -> Value {
    let options = question["criteria"].as_object().unwrap();
    let rest = 0.1 / (options.len() - 1) as f64;
    let probabilities: serde_json::Map<String, Value> = options
        .keys()
        .map(|name| (name.clone(), json!(if name == option { 0.9 } else { rest })))
        .collect();
    json!({"type": "choice", "choice": option, "confidence": 0.9, "probabilities": probabilities})
}

#[test]
fn a_convention_proposed_from_agents_md_and_accepted_gates_a_pull_request() {
    let project = Project::new();
    std::fs::write(project.0.join("AGENTS.md"), AGENTS).unwrap();
    std::fs::create_dir(project.0.join("src")).unwrap();
    std::fs::write(project.0.join("src/lib.rs"), JUDGED_RS).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "base"]);
    let provider = scripted();
    let jevgate = |args: &[&str]| {
        let output = project
            .command()
            .args(args)
            .env("TYPESAFE_API_KEY", "key")
            .env("JEVGATE_BASE_URL", &provider.url)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr);
        (output.status.code(), text)
    };

    let (code, text) = jevgate(&["rules", "propose"]);
    assert_eq!(code, Some(0), "{text}");
    let proposal = project
        .0
        .join(".jevgate/proposals/never-log-request-bodies.toml");
    let proposed = std::fs::read_to_string(&proposal).unwrap();
    assert!(
        proposed.contains("the project rule \"Never log request bodies.\" (AGENTS.md:3)?"),
        "it quotes and cites the line: {proposed}"
    );
    assert!(proposed.contains("level = \"note\"\n"), "{proposed}");
    assert!(
        proposed.contains("run `jevgate rules test --rule custom/never-log-request-bodies`"),
        "it says to add guidance and examples and test them first: {proposed}"
    );
    let reviewed = proposed.replace(
        "level = \"note\"\n",
        &format!("level = \"review\"\n{GUIDANCE}"),
    ) + EXAMPLES;
    std::fs::write(&proposal, reviewed).unwrap();
    let (code, text) = jevgate(&["rules", "accept", "never-log-request-bodies"]);
    assert_eq!(code, Some(0), "{text}");
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "Accept a question"]);
    let tracked = Command::new("git")
        .arg("-C")
        .arg(&project.0)
        .args(["ls-files", ".jevgate"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&tracked.stdout),
        ".jevgate/questions/never-log-request-bodies.toml\n",
        "the accepted question is committed, the cache is not"
    );

    let (code, text) = jevgate(&["rules", "test", "--format", "json"]);
    assert_eq!(code, Some(0), "its examples pass: {text}");
    // The report on stdout comes first, then what stderr said.
    let tested: Value = serde_json::Deserializer::from_str(&text)
        .into_iter()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(tested["passed"], true, "{tested}");
    let results: Vec<(&str, &str)> = tested["questions"][0]["examples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["expected"].as_str().unwrap(),
                e["result"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(results, [("failing", "right"), ("passing", "right")]);

    let rule = "custom/never-log-request-bodies";
    std::fs::write(
        project.0.join("src/lib.rs"),
        format!("{JUDGED_RS}{BREAKING}"),
    )
    .unwrap();
    let (code, text) = jevgate(&["check", "--base", "HEAD"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(
        text.contains(&format!("src/lib.rs:9 [{rule}] (fails the gate) `charge`")),
        "{text}"
    );
    let fixed = BREAKING.replace("request.body", "request.id");
    std::fs::write(project.0.join("src/lib.rs"), format!("{JUDGED_RS}{fixed}")).unwrap();
    let (code, text) = jevgate(&["check", "--base", "HEAD"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(!text.contains(rule), "{text}");
}
