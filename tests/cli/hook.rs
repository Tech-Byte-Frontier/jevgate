//! `jevgate hook` as agents run it: one event on stdin, one reply on stdout,
//! and exit 0 whatever happens, since agents read exit 2 as a block.
use super::*;
use mock_provider::{MockProvider, Reply, answer};
use serde_json::json;

/// The reply of `jevgate hook` to a Claude Code event of `project`, its
/// checks asking `provider` with `key`.
fn hook_asking(
    project: &Project,
    provider: &MockProvider,
    key: &str,
    fields: serde_json::Value,
) -> serde_json::Value {
    hook(
        project.asking(provider, key),
        &[],
        &hook_event(project, fields),
    )
    .0
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

#[test]
fn without_a_key_every_event_passes_and_says_why() {
    let project = Project::committed();
    let start = serde_json::json!({"hook_event_name": "UserPromptSubmit", "prompt": "go"});
    assert_eq!(
        hook(project.command(), &[], &hook_event(&project, start)).0["hookSpecificOutput"]["additionalContext"],
        RUNNING,
        "a session's first turn says the hooks run"
    );
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS.replace("+ 1", "+ 2")).unwrap();
    let edit = serde_json::json!({"hook_event_name": "PostToolUse", "tool_name": "Edit",
        "tool_input": {"file_path": project.0.join("lib.rs")}});
    let (edited, _) = hook(project.command(), &[], &hook_event(&project, edit));
    let context = edited["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        context.starts_with("JevGate could not check lib.rs (No API key configured."),
        "{context}"
    );
    let stop = serde_json::json!({"hook_event_name": "Stop", "stop_hook_active": false});
    let (stopped, _) = hook(project.command(), &[], &hook_event(&project, stop));
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
fn a_402_or_another_providers_key_blocks_nothing_and_the_person_reads_why() {
    let provider = MockProvider::start(|_| {
        Reply::json(402, &json!({"error": "private"}))
            .header("x-typesafe-request-id", "req_hook402")
    });
    let cases = [
        (
            "key",
            "TypeSafe HTTP 402 (credits exhausted; add credits or turn on auto-refill at https://console.typesafe.ai); request was not retried; request id req_hook402",
        ),
        (
            "sk-or-v1-hook",
            "TYPESAFE_API_KEY environment variable: the key was issued by OpenRouter (it starts with sk-or-), not by TypeSafe; set it as OPENROUTER_API_KEY, or save it with jevgate auth login --provider openrouter",
        ),
    ];
    for (key, said) in cases {
        let project = Project::committed();
        let send = |fields| hook_asking(&project, &provider, key, fields);
        send(json!({"hook_event_name": "UserPromptSubmit", "prompt": "add the spread"}));
        std::fs::write(project.0.join("lib.rs"), LONG_RS).unwrap();
        let stopped = send(json!({"hook_event_name": "Stop", "stop_hook_active": false}));
        assert_eq!(
            stopped,
            json!({"systemMessage": format!("JevGate could not check this turn: {said}. Nothing was blocked.")}),
            "{key}"
        );
        assert!(
            !project.0.join(".jevgate/turns/outage.json").exists(),
            "{key}: waiting would not fix it, so the next event asks again"
        );
    }
    let received = provider.received();
    assert!(!received.is_empty());
    assert!(
        received
            .iter()
            .all(|r| r.header("authorization") == Some("Bearer key")),
        "another provider's key is never sent"
    );
}

#[test]
fn cursor_is_answered_in_its_own_fields_and_the_person_on_stderr() {
    let project = Project::new();
    let start = serde_json::json!({"hook_event_name": "beforeSubmitPrompt", "prompt": "go"});
    let (reply, stderr) = hook(
        project.command(),
        &["--agent", "cursor"],
        &hook_event(&project, start),
    );
    assert_eq!(reply, serde_json::json!({"continue": true}));
    assert!(stderr.contains("is not in a Git repository"), "{stderr}");
}

#[test]
fn the_binary_blocks_a_turn_through_the_provider_until_its_finding_is_fixed() {
    let project = Project::committed();
    // Jev's part, scripted: a concern only about the long function.
    let provider = MockProvider::start(|received| {
        let level = if received.body.contains("smallest") {
            2
        } else {
            0
        };
        Reply::json(200, &answer(&received.json(), level))
    });
    let send = |fields| hook_asking(&project, &provider, "key", fields);
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
        "JevGate: the findings that blocked this turn are fixed or dismissed."
    );
}
