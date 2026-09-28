//! A Claude Code session replayed from what Claude Code writes to the hook's
//! stdin (`session/*.json`), with Jev's part scripted: the person asks for a
//! function, the agent writes one that mixes separate jobs, the end of the
//! turn is blocked on that finding, the agent splits the function, and the
//! turn ends. `session/replies.jsonl` holds what `jevgate hook` printed for
//! each event; after changing what the hook says, rerun with
//! `JEVGATE_WRITE_SESSION=1` to rewrite it, and read the diff.
use super::*;
use std::path::Path;

/// The session's events in the order Claude Code sends them. `$PROJECT` is
/// the repository. `tool_response` keeps the fields that name the file;
/// Claude Code also sends the patch and the file's earlier text, which
/// JevGate never reads.
const EVENTS: [(&str, &str); 6] = [
    (
        "1-session-start",
        include_str!("session/1-session-start.json"),
    ),
    ("2-prompt", include_str!("session/2-prompt.json")),
    ("3-write", include_str!("session/3-write.json")),
    ("4-stop", include_str!("session/4-stop.json")),
    ("5-edit", include_str!("session/5-edit.json")),
    ("6-stop", include_str!("session/6-stop.json")),
];

/// What `jevgate hook` printed for each event, one line per event.
const REPLIES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/hook/tests/session/replies.jsonl"
);

/// Stands for the repository in the fixtures' paths.
const PROJECT: &str = "$PROJECT";

/// The order types the agent's code imports, committed before the session.
const ORDER_TS: &str = r#"export interface LineItem {
  sku: string;
  name: string;
  quantity: number;
  unitPrice: number;
}

export interface Order {
  id: string;
  region: "EU" | "UK" | "US";
  items: LineItem[];
}
"#;

/// The scripted answer's line count past which a function mixes separate jobs.
const LONG_FUNCTION_LINES: usize = 20;

/// The scripted split answer: "No", "Slightly" and "Yes" (the function mixes
/// separate jobs), a review at 0.80 or more.
const SPLIT: [f64; 3] = [0.03, 0.06, 0.91];

/// Jev's part, scripted: the split question of a function longer than
/// [`LONG_FUNCTION_LINES`] is answered [`SPLIT`], every other question at the
/// bottom of its scale, and a Choice with `none` where it is offered.
struct Script;

impl Evaluator for Script {
    fn evaluate(&mut self, request: &Value) -> anyhow::Result<Value> {
        let mut reply = answer(request, 0);
        for (key, answer) in reply["answers"].as_object_mut().into_iter().flatten() {
            if asks_to_split_a_long_function(request, key) {
                let [no, slightly, yes] = SPLIT;
                // A Score's score is its probability-weighted level.
                *answer = json!({"type": "score", "score": slightly + 2.0 * yes, "confidence": yes,
                    "probabilities": {"0": no, "1": slightly, "2": yes}});
            }
        }
        Ok(reply)
    }
}

/// Whether question `key` (`f0_split` asks about `functions[0]`) is the
/// split question of a function longer than [`LONG_FUNCTION_LINES`].
fn asks_to_split_a_long_function(request: &Value, key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|rest| rest.strip_suffix("_split"))
        .and_then(|index| index.parse::<usize>().ok())
        .and_then(|index| request["state"]["functions"][index]["source"].as_str())
        .is_some_and(|source| source.lines().count() > LONG_FUNCTION_LINES)
}

/// The repository before the session: its README and the order types,
/// committed, and no `jevgate.toml`: the default rules and gate.
fn shop() -> Project {
    let project = Project::new();
    project.write("README.md", "# shop\n\nOrders and their receipts.\n");
    project.write("src/order.ts", ORDER_TS);
    project.git(&["init", "-q"]);
    project.git(&["add", "."]);
    project.git(&["commit", "-qm", "Order types"]);
    project
}

/// `value` with each string that starts with `$PROJECT` made a path in
/// `root`, joined a component at a time: Claude Code sends native
/// separators, and a canonical Windows path (`\\?\C:\…`) reads no `/`.
fn locate(value: &mut Value, root: &Path) {
    match value {
        Value::String(text) => {
            if let Some(rest) = text.strip_prefix(PROJECT) {
                let path = rest
                    .split('/')
                    .filter(|part| !part.is_empty())
                    .fold(root.to_path_buf(), |path, part| path.join(part));
                *text = path.to_str().unwrap().to_string();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| locate(item, root)),
        Value::Object(fields) => fields.values_mut().for_each(|item| locate(item, root)),
        _ => {}
    }
}

/// Carry out an event's Write or Edit, as Claude Code does before the hook
/// runs; other events change nothing.
fn apply(event: &Value) {
    let input = &event["tool_input"];
    let Some(path) = input["file_path"].as_str() else {
        return;
    };
    let text = |key: &str| input[key].as_str().unwrap();
    match event["tool_name"].as_str() {
        Some("Write") => std::fs::write(path, text("content")).unwrap(),
        Some("Edit") => {
            let before = std::fs::read_to_string(path).unwrap();
            assert_eq!(before.matches(text("old_string")).count(), 1, "{path}");
            let after = before.replacen(text("old_string"), text("new_string"), 1);
            std::fs::write(path, after).unwrap();
        }
        _ => {}
    }
}

/// Claude Code's part for one event: its tool call carried out, then the
/// event written to the hook's stdin; the hook's reply.
fn play(project: &Project, host: &Host, fixture: &str) -> Value {
    let mut event: Value = serde_json::from_str(fixture).unwrap();
    locate(&mut event, &project.0);
    apply(&event);
    let stdin = serde_json::to_vec(&event).unwrap();
    let input = read_event(stdin.as_slice()).unwrap();
    respond(&input, Options::default(), host).json
}

#[test]
fn a_replayed_claude_code_session_blocks_until_the_agent_splits_its_function() {
    let project = shop();
    let host = host(|| Box::new(Script));
    let replies: Vec<Value> = EVENTS
        .iter()
        .map(|(_, fixture)| play(&project, &host, fixture))
        .collect();
    let blocked: Vec<&str> = EVENTS
        .iter()
        .zip(&replies)
        .filter(|(_, reply)| reply["decision"] == "block")
        .map(|((name, _), _)| *name)
        .collect();
    assert_eq!(blocked, ["4-stop"], "only the first stop blocks");
    let last = replies.last().unwrap();
    assert_eq!(
        message(last),
        "JevGate: the findings that blocked this turn are fixed.",
        "{last}"
    );
    // One line per event, as `jevgate hook` prints each reply.
    let printed: Vec<String> = replies.iter().map(Value::to_string).collect();
    if std::env::var_os("JEVGATE_WRITE_SESSION").is_some() {
        std::fs::write(REPLIES, printed.join("\n") + "\n").unwrap();
    }
    // Git may check the file out with CRLF line ends on Windows.
    let expected = std::fs::read_to_string(REPLIES)
        .unwrap_or_default()
        .replace('\r', "");
    let expected: Vec<&str> = expected.lines().collect();
    for (step, ((name, _), reply)) in EVENTS.iter().zip(&printed).enumerate() {
        assert_eq!(
            expected.get(step).copied(),
            Some(reply.as_str()),
            "{name}: the hook's reply changed; if that is intended, rerun with JEVGATE_WRITE_SESSION=1 and read the diff of {REPLIES}"
        );
    }
    assert_eq!(expected.len(), EVENTS.len(), "one reply per event");
}
