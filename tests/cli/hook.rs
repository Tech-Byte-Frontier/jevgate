//! `jevgate hook` as agents run it: one event on stdin, one reply on stdout,
//! and exit 0 whatever happens, since agents read exit 2 as a block.
use super::*;
use mock_provider::{MockProvider, Reply, answer};
use serde_json::json;
use std::io::Write;

/// Run `jevgate hook ARGS`, started by `command`, with `event` on stdin; its
/// reply, its stderr, and that it exited 0.
fn hook(mut command: Command, args: &[&str], event: &str) -> (serde_json::Value, String) {
    let mut child = command
        .arg("hook")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(event.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    let reply = serde_json::from_slice(&output.stdout).unwrap();
    (reply, stderr)
}

/// A Claude Code event of one session in `project`.
fn event(project: &Project, fields: serde_json::Value) -> String {
    let mut event = fields;
    event["session_id"] = "cli-session".into();
    event["cwd"] = project.0.to_str().unwrap().into();
    event.to_string()
}

#[test]
fn invalid_arguments_and_input_are_answered_with_exit_0() {
    let project = Project::new();
    for (args, input, said) in [
        (&["--agent", "nope"][..], "{}", "hook command is invalid"),
        (&["--timeout", "0"][..], "{}", "hook command is invalid"),
        (&[][..], "not json", "could not read the hook event"),
        (&[][..], "[1, 2]", "not a JSON object"),
    ] {
        let (reply, _) = hook(project.command(), args, input);
        let message = reply["systemMessage"].as_str().unwrap();
        assert!(message.contains(said), "{args:?}: {message}");
        assert!(message.ends_with("Nothing was checked or blocked."));
    }
}

/// What a session's first event tells the agent.
const RUNNING: &str =
    "JevGate's hooks run in this session: they check each edit and the end of each turn.";

/// A Git repository with a short function committed as `lib.rs`.
fn committed() -> Project {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "lib.rs"]);
    git(&project, &["commit", "-qm", "start"]);
    project
}

#[test]
fn without_a_key_every_event_passes_and_says_why() {
    let project = committed();
    let start = serde_json::json!({"hook_event_name": "UserPromptSubmit", "prompt": "go"});
    assert_eq!(
        hook(project.command(), &[], &event(&project, start)).0["hookSpecificOutput"]["additionalContext"],
        RUNNING,
        "a session's first turn says the hooks run"
    );
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS.replace("+ 1", "+ 2")).unwrap();
    let edit = serde_json::json!({"hook_event_name": "PostToolUse", "tool_name": "Edit",
        "tool_input": {"file_path": project.0.join("lib.rs")}});
    let (edited, _) = hook(project.command(), &[], &event(&project, edit));
    let context = edited["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        context.starts_with("JevGate could not check lib.rs (No API key configured."),
        "{context}"
    );
    let stop = serde_json::json!({"hook_event_name": "Stop", "stop_hook_active": false});
    let (stopped, _) = hook(project.command(), &[], &event(&project, stop));
    assert!(stopped.get("decision").is_none(), "{stopped}");
    assert!(
        stopped["systemMessage"]
            .as_str()
            .unwrap()
            .starts_with("JevGate could not check this turn: No API key configured."),
        "{stopped}"
    );
}

#[test]
fn cursor_is_answered_in_its_own_fields_and_the_person_on_stderr() {
    let project = Project::new();
    let start = serde_json::json!({"hook_event_name": "beforeSubmitPrompt", "prompt": "go"});
    let (reply, stderr) = hook(
        project.command(),
        &["--agent", "cursor"],
        &event(&project, start),
    );
    assert_eq!(reply, serde_json::json!({"continue": true}));
    assert!(stderr.contains("is not in a Git repository"), "{stderr}");
}

/// A function longer than twenty lines, which a split question at the top
/// of its scale makes a function-simplification review.
const LONG_RS: &str = "fn f(values: &[i32]) -> i32 {\n    let mut total = 0;\n    for value in values {\n        total += value;\n    }\n    let mut largest = i32::MIN;\n    for value in values {\n        if *value > largest {\n            largest = *value;\n        }\n    }\n    let mut smallest = i32::MAX;\n    for value in values {\n        if *value < smallest {\n            smallest = *value;\n        }\n    }\n    let spread = largest - smallest;\n    let doubled = total * 2;\n    doubled + spread + 1\n}\n";

#[test]
fn the_binary_blocks_a_turn_through_the_provider_until_its_finding_is_fixed() {
    let project = committed();
    // Jev's part, scripted: a concern only about the long function.
    let provider = MockProvider::start(|received| {
        let level = if received.body.contains("smallest") {
            2
        } else {
            0
        };
        Reply::json(200, &answer(&received.json(), level))
    });
    let send = |fields: serde_json::Value| {
        let mut command = project.command();
        command
            .env("TYPESAFE_API_KEY", "key")
            .env("JEVGATE_BASE_URL", &provider.url);
        hook(command, &[], &event(&project, fields)).0
    };
    let prompt = json!({"hook_event_name": "UserPromptSubmit", "prompt": "add the spread"});
    assert_eq!(
        send(prompt)["hookSpecificOutput"]["additionalContext"],
        RUNNING
    );
    std::fs::write(project.0.join("lib.rs"), LONG_RS).unwrap();
    let blocked = send(json!({"hook_event_name": "Stop", "stop_hook_active": false}));
    assert_eq!(blocked["decision"], "block", "{blocked}");
    let reason = blocked["reason"].as_str().unwrap();
    assert!(
        reason.contains(
            "\n- lib.rs:1 review maintainability/function-simplification (fails the gate): "
        ),
        "{reason}"
    );
    assert!(
        !provider.received().is_empty(),
        "asked through the provider"
    );
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS.replace("+ 1", "+ 2")).unwrap();
    let fixed = send(json!({"hook_event_name": "Stop", "stop_hook_active": true}));
    assert!(fixed.get("decision").is_none(), "{fixed}");
    assert_eq!(
        fixed["systemMessage"],
        "JevGate: the findings that blocked this turn are fixed."
    );
}
