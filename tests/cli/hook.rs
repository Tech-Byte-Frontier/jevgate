//! `jevgate hook` as agents run it: one event on stdin, one reply on stdout,
//! and exit 0 whatever happens, since agents read exit 2 as a block.
use super::*;
use std::io::Write;

/// Run `jevgate hook ARGS` with `event` on stdin; its reply, its stderr, and
/// that it exited 0.
fn hook(project: &Project, args: &[&str], event: &str) -> (serde_json::Value, String) {
    let mut child = project
        .command()
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
        let (reply, _) = hook(&project, args, input);
        let message = reply["systemMessage"].as_str().unwrap();
        assert!(message.contains(said), "{args:?}: {message}");
        assert!(message.ends_with("Nothing was checked or blocked."));
    }
}

#[test]
fn without_a_key_every_event_passes_and_says_why() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "lib.rs"]);
    git(&project, &["commit", "-qm", "start"]);
    let start = serde_json::json!({"hook_event_name": "UserPromptSubmit", "prompt": "go"});
    assert_eq!(
        hook(&project, &[], &event(&project, start)).0,
        serde_json::json!({})
    );
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS.replace("+ 1", "+ 2")).unwrap();
    let edit = serde_json::json!({"hook_event_name": "PostToolUse", "tool_name": "Edit",
        "tool_input": {"file_path": project.0.join("lib.rs")}});
    let (edited, _) = hook(&project, &[], &event(&project, edit));
    let context = edited["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        context.starts_with("JevGate could not check lib.rs (No API key configured."),
        "{context}"
    );
    let stop = serde_json::json!({"hook_event_name": "Stop", "stop_hook_active": false});
    let (stopped, _) = hook(&project, &[], &event(&project, stop));
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
    let (reply, stderr) = hook(&project, &["--agent", "cursor"], &event(&project, start));
    assert_eq!(reply, serde_json::json!({"continue": true}));
    assert!(stderr.contains("is not in a Git repository"), "{stderr}");
}
