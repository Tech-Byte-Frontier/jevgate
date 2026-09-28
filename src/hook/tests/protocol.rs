//! What each agent sends and reads: detection, event names, edited files,
//! reply shapes, and the size of a reply.
use super::super::{agents, review::Flagged};
use super::*;
use crate::schema::Strength;

fn event(agent: Agent, input: Value) -> agents::Event {
    agents::event(agent, &input)
}

#[test]
fn each_agent_is_detected_from_what_only_it_sends() {
    let cases = [
        (
            json!({"hook_event_name": "Stop", "session_id": "s", "stop_hook_active": false}),
            Agent::Claude,
        ),
        (
            json!({"hook_event_name": "Stop", "session_id": "s", "turn_id": "t"}),
            Agent::Codex,
        ),
        (
            json!({"hook_event_name": "AfterAgent", "timestamp": "2026-09-28T01:00:00Z"}),
            Agent::Gemini,
        ),
        (
            json!({"hook_event_name": "stop", "conversation_id": "c", "cursor_version": "1.7.2"}),
            Agent::Cursor,
        ),
        (
            json!({"hook_event_name": "Stop", "conversation_id": "c"}),
            Agent::Cursor,
        ),
        (
            json!({"hook_event_name": "session.idle", "sessionID": "o"}),
            Agent::Opencode,
        ),
        (
            json!({"hook_event_name": "Stop", "session_id": "s", "timestamp": "2026-09-28T01:00:00Z"}),
            Agent::Copilot,
        ),
        (
            json!({"hook_event_name": "Stop", "timestamp": "2026-09-28T01:00:00Z",
                "permission_mode": "default"}),
            Agent::Claude,
        ),
    ];
    for (input, agent) in cases {
        assert_eq!(agents::detect(&input), agent, "{input}");
    }
}

#[test]
fn every_agents_names_for_an_event_are_read_whoever_sends_them() {
    use agents::Kind::*;
    let cases = [
        ("SessionStart", "", SessionStart),
        ("sessionStart", "", SessionStart),
        ("UserPromptSubmit", "", TurnStart),
        ("BeforeAgent", "", TurnStart),
        ("beforeSubmitPrompt", "", TurnStart),
        ("PostToolUse", "Edit", AfterEdit),
        ("PostToolUse", "MultiEdit", AfterEdit),
        ("PostToolUse", "NotebookEdit", AfterEdit),
        ("PostToolUse", "apply_patch", AfterEdit),
        ("AfterTool", "write_file", AfterEdit),
        ("AfterTool", "replace", AfterEdit),
        ("postToolUse", "Write", AfterEdit),
        ("PostToolUse", "Bash", Other),
        ("AfterTool", "read_file", Other),
        ("Stop", "", Stop),
        ("AfterAgent", "", Stop),
        ("stop", "", Stop),
        ("SubagentStop", "", Other),
        ("PreToolUse", "Edit", Other),
    ];
    for (name, tool, kind) in cases {
        let input = json!({"hook_event_name": name, "tool_name": tool});
        assert_eq!(event(Agent::Claude, input).kind, kind, "{name} {tool}");
    }
    let opencode = |name: &str, tool: &str| {
        event(
            Agent::Opencode,
            json!({"hook_event_name": name, "tool": tool, "args": {"filePath": "src/a.ts"}}),
        )
    };
    assert_eq!(opencode("session.created", "").kind, SessionStart);
    assert_eq!(opencode("chat.message", "").kind, TurnStart);
    assert_eq!(opencode("tool.execute.after", "edit").kind, AfterEdit);
    assert_eq!(opencode("tool.execute.after", "read").kind, Other);
    assert_eq!(opencode("session.idle", "").kind, Stop);
    assert_eq!(
        opencode("tool.execute.after", "write").files,
        [PathBuf::from("src/a.ts")]
    );
}

#[test]
fn edited_files_are_read_from_each_agents_tool_input() {
    let files = |agent: Agent, tool: &str, input: Value| {
        event(
            agent,
            json!({"hook_event_name": "PostToolUse", "tool_name": tool, "tool_input": input}),
        )
        .files
    };
    assert_eq!(
        files(
            Agent::Claude,
            "Edit",
            json!({"file_path": "/r/src/a.rs", "old_string": "a"})
        ),
        [PathBuf::from("/r/src/a.rs")]
    );
    assert_eq!(
        files(
            Agent::Claude,
            "NotebookEdit",
            json!({"notebook_path": "/r/n.ipynb"})
        ),
        [PathBuf::from("/r/n.ipynb")]
    );
    let patch = "*** Begin Patch\n*** Add File: src/new.rs\n+fn a() {}\n*** Update File: src/old.rs\n*** Move to: src/moved.rs\n@@\n-fn b() {}\n+fn c() {}\n *** Update File: quoted.rs\n*** Delete File: src/gone.rs\n*** End Patch\n";
    assert_eq!(
        files(Agent::Codex, "apply_patch", json!({"command": patch})),
        [
            PathBuf::from("src/new.rs"),
            PathBuf::from("src/old.rs"),
            PathBuf::from("src/moved.rs")
        ],
        "a hunk's context line quoting a header is not a path"
    );
    let opencode = event(
        Agent::Opencode,
        json!({"hook_event_name": "tool.execute.after", "tool": "patch", "args": {"patchText": patch}}),
    );
    assert_eq!(opencode.files.len(), 3);
    assert!(files(Agent::Claude, "Edit", json!({})).is_empty());
}

#[test]
fn a_stop_is_blocked_through_each_agents_own_fields() {
    let reply = agents::Reply {
        block: true,
        agent: Some("Fix it.".into()),
        user: Some("Told.".into()),
    };
    let claude = json!({"decision": "block", "reason": "Fix it.", "systemMessage": "Told."});
    let cases = [
        (Agent::Claude, "Stop", claude.clone()),
        (Agent::Codex, "Stop", claude.clone()),
        (Agent::Gemini, "AfterAgent", claude.clone()),
        (Agent::Opencode, "session.idle", claude),
        (
            Agent::Cursor,
            "stop",
            json!({"followup_message": "Fix it."}),
        ),
        (
            Agent::Copilot,
            "Stop",
            json!({"decision": "block", "reason": "Fix it.", "systemMessage": "Told.",
                "hookSpecificOutput": {"hookEventName": "Stop", "decision": "block", "reason": "Fix it."}}),
        ),
    ];
    for (agent, name, expected) in cases {
        let stop = event(agent, json!({"hook_event_name": name}));
        assert_eq!(agents::render(&stop, &reply), expected, "{agent:?}");
    }
}

#[test]
fn context_goes_only_where_it_does_not_continue_a_stopped_turn() {
    let reply = agents::Reply {
        block: false,
        agent: Some("Facts.".into()),
        user: Some("Told.".into()),
    };
    let render = |agent: Agent, name: &str| {
        let input = json!({"hook_event_name": name, "tool_name": "Edit", "tool": "edit"});
        agents::render(&event(agent, input), &reply)
    };
    assert_eq!(
        render(Agent::Claude, "PostToolUse"),
        json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": "Facts."},
            "systemMessage": "Told."})
    );
    assert_eq!(
        render(Agent::Gemini, "AfterTool")["hookSpecificOutput"]["hookEventName"],
        "AfterTool"
    );
    assert_eq!(
        render(Agent::Cursor, "postToolUse"),
        json!({"additional_context": "Facts."})
    );
    assert_eq!(
        render(Agent::Cursor, "beforeSubmitPrompt"),
        json!({"continue": true})
    );
    let copilot = render(Agent::Copilot, "PostToolUse");
    assert_eq!(copilot["additionalContext"], "Facts.");
    assert_eq!(copilot["hookSpecificOutput"]["additionalContext"], "Facts.");
    // Codex rejects any field its Stop output does not define.
    let allowed = [
        "continue",
        "stopReason",
        "systemMessage",
        "suppressOutput",
        "decision",
        "reason",
    ];
    for agent in [Agent::Claude, Agent::Codex] {
        let stop = render(agent, "Stop");
        assert_eq!(stop, json!({"systemMessage": "Told."}), "{agent:?}");
        assert!(
            stop.as_object()
                .unwrap()
                .keys()
                .all(|k| allowed.contains(&k.as_str()))
        );
    }
}

/// `n` findings that fail the gate, each with a long why and next step.
fn failing(n: usize, words: usize) -> Vec<Flagged> {
    (0..n)
        .map(|i| Flagged {
            path: PathBuf::from(format!("src/module_{i}.rs")),
            finding: crate::schema::Finding {
                line: i + 1,
                message: "word ".repeat(words),
                action: "step ".repeat(words),
                gate: Some(crate::schema::Gating::Fails),
                ..crate::tests::finding(Strength::Review)
            },
            accepted_this_turn: false,
        })
        .collect()
}

#[test]
fn a_reply_lists_ten_findings_one_line_each_in_under_8000_characters() {
    let reason = text::block_reason(&failing(30, 200), 1);
    assert!(reason.chars().count() < 8_000, "{}", reason.len());
    let lines: Vec<&str> = reason.lines().filter(|l| l.starts_with("- ")).collect();
    assert_eq!(lines.len(), 10);
    assert!(lines.iter().all(|l| l.chars().count() < 600), "clipped");
    assert!(lines[0].starts_with(
        "- src/module_0.rs:1 review maintainability/shared-logic (fails the gate): word word"
    ));
    assert!(
        reason.contains("\n20 more findings not shown; .jevgate/latest.json holds every finding")
    );
    assert!(
        reason.contains("\nFix them, then finish. If a finding is mistaken, keep the code"),
        "{reason}"
    );
    let short = text::after_edit(&[PathBuf::from("src/a.rs")], &failing(2, 3), &[], &[]).unwrap();
    assert!(!short.contains("not shown"), "{short}");
    // Guards take their room first: the whole context still fits.
    let guards: Vec<crate::guards::Guard> = (0..12)
        .map(|n| {
            serde_json::from_value(serde_json::json!({
                "kind": "suppression", "path": format!("src/g{n}.py"), "line": n + 1,
                "text": "x = 1  # noqa: E501 ".repeat(20), "message": "turns off flake8 or Ruff here", "id": n.to_string()
            }))
            .unwrap()
        })
        .collect();
    let guards: Vec<&crate::guards::Guard> = guards.iter().collect();
    let full = text::after_edit(
        &[PathBuf::from("src/a.rs")],
        &failing(30, 200),
        &[],
        &guards,
    )
    .unwrap();
    assert!(full.chars().count() < 8_000, "{}", full.len());
    assert!(
        full.contains("more findings not shown")
            && full.ends_with("as they were when the turn began."),
        "{full}"
    );
    let flagged = Flagged {
        path: PathBuf::from("src/a,b.rs"),
        finding: crate::tests::finding(Strength::Consider),
        accepted_this_turn: false,
    };
    let file = [PathBuf::from("src/a,b.rs")];
    assert_eq!(
        text::after_edit(&file, std::slice::from_ref(&flagged), &[], &[]).unwrap(),
        "JevGate reviewed src/a,b.rs after this edit: 1 finding, none fails the quality gate.\n- src/a,b.rs:12 consider maintainability/shared-logic: Copies: 50% alike, see `b`. Next: Share one | implementation.\nNone of them blocks the end of the turn."
    );
    assert_eq!(
        text::after_edit(&file, &[], &failing(2, 3), &[]).unwrap(),
        "JevGate reviewed src/a,b.rs after this edit: 2 findings reported earlier this turn remain (2 fail the quality gate)."
    );
    let both = text::after_edit(&file, &[flagged], &failing(1, 3), &[]).unwrap();
    assert!(both.starts_with("JevGate reviewed src/a,b.rs after this edit: 1 new finding, none fails the quality gate.\n- "), "{both}");
    assert!(both.ends_with("\n1 finding reported earlier this turn remains (1 fails the quality gate).\nFindings that fail the gate block the end of the turn until they are fixed; the others are optional."), "{both}");
    assert_eq!(text::after_edit(&file, &[], &[], &[]), None);
}
