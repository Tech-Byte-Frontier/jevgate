//! The hook end to end in Git repositories, with scripted evaluators: turn
//! baselines, findings after an edit, blocks at the end of a turn and their
//! limits, and every way a check can fail without blocking the agent. What
//! each agent sends and reads is in `protocol`; a whole Claude Code session,
//! replayed from its events, is in `session`.
use super::*;
use crate::{
    provider::TYPESAFE,
    provider_error::{Failure, Unsent, provider_error},
    tests::{Mock, Project, answer, function, long_function},
    transport::Evaluator,
};
mod guards;
mod protocol;
mod session;

/// A Git repository with `lib.rs` committed, judged for function
/// simplification only: a long `lib.rs` is a review that fails the gate
/// with answers at level 2, a short one a note.
fn repository() -> Project {
    let project = Project::new();
    project.write("jevgate.toml", "rules = [\"function-simplification\"]\n");
    project.write("lib.rs", &function("f"));
    project.git(&["init", "-q"]);
    project.git(&["add", "."]);
    project.git(&["commit", "-qm", "start"]);
    project
}

/// Checks answered by `evaluator`, one per check.
fn host(evaluator: impl Fn() -> Box<dyn Evaluator + Send> + Send + Sync + 'static) -> Host {
    Host {
        evaluators: Arc::new(move |_, _| Ok(evaluator())),
        cwd: PathBuf::from("/nonexistent"),
    }
}

/// Every answer at the top of its scale: a long function is a review.
fn reviewing() -> Host {
    host(|| {
        Box::new(Mock {
            level: 2,
            ..Default::default()
        })
    })
}

/// A provider that fails every request with its error.
#[derive(Clone, Copy)]
struct Failing(fn() -> anyhow::Error);

impl Evaluator for Failing {
    fn evaluate(&mut self, _: &Value) -> anyhow::Result<Value> {
        Err((self.0)())
    }
}

/// A provider with a concern only about a function named `old`: a check
/// that asks about `old` reports it, and one that does not asks nothing
/// that finds anything.
struct AboutOld;

impl Evaluator for AboutOld {
    fn evaluate(&mut self, request: &Value) -> anyhow::Result<Value> {
        let level = if request.to_string().contains("fn old(") {
            2
        } else {
            0
        };
        Ok(answer(request, level))
    }
}

/// A provider that answers only after `0`.
struct Slow(Duration);

impl Evaluator for Slow {
    fn evaluate(&mut self, request: &Value) -> anyhow::Result<Value> {
        std::thread::sleep(self.0);
        Ok(answer(request, 0))
    }
}

/// `event` as an agent sends it from `project` in session `session`.
fn from(project: &Project, session: &str, mut event: Value) -> Value {
    event["session_id"] = json!(session);
    event["cwd"] = json!(project.0.as_path());
    event
}

/// One Claude Code event of a session in `project`, and the reply.
fn send(project: &Project, host: &Host, event: Value) -> Value {
    respond(&from(project, "session-1", event), Options::default(), host).json
}

/// The same with a time budget.
fn send_within(project: &Project, host: &Host, event: Value, budget: Duration) -> Value {
    let options = Options {
        timeout: Some(budget),
        ..Options::default()
    };
    respond(&from(project, "session-1", event), options, host).json
}

fn prompt(text: &str) -> Value {
    json!({"hook_event_name": "UserPromptSubmit", "prompt": text})
}

fn edit(project: &Project, file: &str) -> Value {
    json!({"hook_event_name": "PostToolUse", "tool_name": "Edit",
        "tool_input": {"file_path": project.0.join(file)}})
}

fn stop(continued: bool) -> Value {
    json!({"hook_event_name": "Stop", "stop_hook_active": continued})
}

/// The text a reply gives the agent as context.
fn context(reply: &Value) -> &str {
    reply["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
}

fn message(reply: &Value) -> &str {
    reply["systemMessage"].as_str().unwrap_or_default()
}

#[test]
fn an_edit_gets_its_findings_as_context_and_never_blocks() {
    let project = repository();
    let host = reviewing();
    assert_eq!(send(&project, &host, prompt("refactor")), json!({}));
    project.write("lib.rs", &long_function("f"));
    let reply = send(&project, &host, edit(&project, "lib.rs"));
    assert!(reply.get("decision").is_none(), "{reply}");
    let text = context(&reply);
    assert!(
        text.starts_with("JevGate reviewed lib.rs after this edit: 1 finding, 1 fails the quality gate.\n- lib.rs:1 review maintainability/function-simplification (fails the gate): "),
        "{text}"
    );
    assert!(text.contains(" Next: "), "{text}");
    project.write("lib.rs", &function("f"));
    let clean = send(&project, &host, edit(&project, "lib.rs"));
    assert_eq!(clean, json!({}), "a clean check says nothing");
    let outside = json!({"hook_event_name": "PostToolUse", "tool_name": "Write",
        "tool_input": {"file_path": std::env::temp_dir().join("elsewhere.rs")}});
    assert_eq!(send(&project, &host, outside), json!({}));
}

#[test]
fn a_baseline_is_not_replaced_by_the_few_files_a_hook_checked() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    send(&project, &host, edit(&project, "lib.rs"));
    let Err(error) = crate::baseline::write(&project.0, false, None) else {
        panic!("a hook's check replaced the baseline");
    };
    assert_eq!(
        error.to_string(),
        "The last check was the agent hook's, of 1 file; run jevgate check first, or accept its findings with --merge, which keeps the rest"
    );
    let merged = crate::baseline::write(&project.0, true, None).unwrap();
    assert_eq!(merged.accepted, 1);
}

#[test]
fn a_finding_is_given_to_the_agent_once_a_turn() {
    let project = repository();
    let host = reviewing();
    // Each edit changes only a comment after `f`, so its finding stays the same.
    let file = |revision: u32| format!("{}// revision {revision}\n", long_function("f"));
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &file(1));
    let first = send(&project, &host, edit(&project, "lib.rs"));
    assert!(context(&first).contains("\n- lib.rs:1 review "), "{first}");
    project.write("lib.rs", &file(2));
    let second = send(&project, &host, edit(&project, "lib.rs"));
    assert_eq!(
        context(&second),
        "JevGate reviewed lib.rs after this edit: 1 finding reported earlier this turn remains (1 fails the quality gate)."
    );
    send(&project, &host, prompt("next task"));
    project.write("lib.rs", &file(3));
    assert_eq!(
        send(&project, &host, edit(&project, "lib.rs")),
        json!({}),
        "this turn changed only the comment after `f`"
    );
    project.write("lib.rs", &file(3).replace("spread + 1", "spread + 2"));
    let next_turn = send(&project, &host, edit(&project, "lib.rs"));
    assert!(
        context(&next_turn).contains("\n- lib.rs:1 review "),
        "a turn that changes `f` is given its finding again: {next_turn}"
    );
}

#[test]
fn a_turn_is_judged_on_what_it_changed_not_on_the_rest_of_its_files() {
    let project = repository();
    let host = host(|| Box::new(AboutOld));
    // `old` would be a review; the turn edits `f` beside it.
    let file = |old: &str, f: &str| format!("{old}\n{f}");
    project.write("lib.rs", &file(&long_function("old"), &function("f")));
    project.commit_all();
    send(&project, &host, prompt("tidy f"));
    let tidied = function("f").replace("total * 2", "total * 3");
    project.write("lib.rs", &file(&long_function("old"), &tidied));
    assert_eq!(
        send(&project, &host, edit(&project, "lib.rs")),
        json!({}),
        "`old` was not touched"
    );
    assert_eq!(send(&project, &host, stop(false)), json!({}));
    send(&project, &host, prompt("now old"));
    let touched = long_function("old").replace("spread + 1", "spread + 2");
    project.write("lib.rs", &file(&touched, &tidied));
    let edited = send(&project, &host, edit(&project, "lib.rs"));
    assert!(
        context(&edited).contains("\n- lib.rs:1 review "),
        "{edited}"
    );
    assert_eq!(send(&project, &host, stop(false))["decision"], "block");
}

#[test]
fn only_findings_that_fail_the_gate_block_the_end_of_a_turn() {
    // Answered "Yes" at 0.6, a long function is a function-simplification
    // consider: reported, but not failing the default gate, which fails only
    // on levels measured right on projects JevGate was never tuned on.
    let host = host(|| {
        Box::new(Mock {
            level: 4,
            ..Default::default()
        })
    });
    let project = repository();
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    let edited = send(&project, &host, edit(&project, "lib.rs"));
    assert!(
        context(&edited)
            .contains("\n- lib.rs:1 consider maintainability/function-simplification: "),
        "{edited}"
    );
    let stopped = send(&project, &host, stop(false));
    assert!(stopped.get("decision").is_none(), "{stopped}");
    assert!(
        message(&stopped).starts_with(
            "JevGate: 1 consider in this turn's changes doesn't fail the quality gate."
        ),
        "{stopped}"
    );
    // A level set in jevgate.toml fails it, from the turn after the edit.
    project.write(
        "jevgate.toml",
        "rules = [\"function-simplification\"]\nfail_on = [\"consider\"]\n",
    );
    send(&project, &host, prompt("again"));
    project.write(
        "lib.rs",
        &long_function("f").replace("spread + 1", "spread + 2"),
    );
    let edited = send(&project, &host, edit(&project, "lib.rs"));
    assert!(
        context(&edited)
            .contains(" consider maintainability/function-simplification (fails the gate): "),
        "{edited}"
    );
    assert_eq!(send(&project, &host, stop(false))["decision"], "block");
}

#[test]
fn a_stop_blocks_until_the_findings_are_fixed() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    let blocked = send(&project, &host, stop(false));
    assert_eq!(blocked["decision"], "block");
    let reason = blocked["reason"].as_str().unwrap();
    assert!(
        reason.starts_with("JevGate blocked the end of this turn (1 of at most 3): 1 finding in code changed this turn fails the quality gate.\n- lib.rs:1 review "),
        "{reason}"
    );
    assert!(reason.ends_with("JevGate does not block again when nothing changed."));
    assert!(message(&blocked).contains("(block 1 of 3)"), "{blocked}");
    assert!(blocked.get("hookSpecificOutput").is_none());
    project.write("lib.rs", &function("f"));
    let fixed = send(&project, &host, stop(true));
    assert!(fixed.get("decision").is_none(), "{fixed}");
    assert_eq!(
        message(&fixed),
        "JevGate: the findings that blocked this turn are fixed."
    );
    assert_eq!(
        send(&project, &host, stop(false)),
        json!({}),
        "the next turn starts from the fix"
    );
}

#[test]
fn a_turn_is_blocked_three_times_at_most() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    for block in 1..=3 {
        project.write("lib.rs", &long_function(&format!("f{block}")));
        let reply = send(&project, &host, stop(block > 1));
        let reason = reply["reason"].as_str().unwrap_or_default();
        assert!(
            reason.contains(&format!("({block} of at most 3)")),
            "{reply}"
        );
    }
    project.write("lib.rs", &long_function("f4"));
    let reply = send(&project, &host, stop(true));
    assert!(reply.get("decision").is_none(), "{reply}");
    assert!(
        message(&reply).starts_with("JevGate blocked this turn 3 times and lets the agent finish; 1 finding still fails the quality gate."),
        "{reply}"
    );
}

#[test]
fn a_stop_the_agent_did_not_continue_counts_blocks_again() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    send(&project, &host, stop(false));
    project.write("lib.rs", &long_function("g"));
    let reply = send(&project, &host, stop(false));
    assert!(
        reply["reason"]
            .as_str()
            .unwrap()
            .contains("(1 of at most 3)"),
        "{reply}"
    );
}

#[test]
fn nothing_changed_after_a_block_lets_the_agent_finish() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    assert_eq!(send(&project, &host, stop(false))["decision"], "block");
    let reply = send(&project, &host, stop(true));
    assert!(reply.get("decision").is_none(), "{reply}");
    assert!(
        message(&reply).starts_with(
            "JevGate lets the agent finish: nothing changed after its last block, and 1 finding still fails the quality gate."
        ),
        "{reply}"
    );
}

#[test]
fn a_block_reason_sent_back_as_a_prompt_continues_the_turn() {
    let project = repository();
    let host = reviewing();
    let gemini = |event: Value| {
        let mut event = from(&project, "gemini-1", event);
        event["timestamp"] = json!("2026-09-28T01:00:00Z");
        respond(&event, Options::default(), &host).json
    };
    gemini(json!({"hook_event_name": "BeforeAgent", "prompt": "refactor"}));
    project.write("lib.rs", &long_function("f"));
    let blocked = gemini(json!({"hook_event_name": "AfterAgent", "stop_hook_active": false}));
    assert_eq!(blocked["decision"], "block");
    let reason = blocked["reason"].as_str().unwrap();
    gemini(json!({"hook_event_name": "BeforeAgent", "prompt": reason}));
    project.write("lib.rs", &long_function("g"));
    let again = gemini(json!({"hook_event_name": "AfterAgent", "stop_hook_active": true}));
    assert!(
        again["reason"]
            .as_str()
            .unwrap()
            .contains("(2 of at most 3)"),
        "the continuation kept the turn's start and its blocks: {again}"
    );
    gemini(json!({"hook_event_name": "BeforeAgent", "prompt": "a new task"}));
    let fresh = gemini(json!({"hook_event_name": "AfterAgent", "stop_hook_active": false}));
    assert_eq!(fresh, json!({}), "a new prompt starts a new turn");
}

#[test]
fn without_a_turn_start_an_edit_is_checked_whole_and_a_stop_starts_one() {
    let project = repository();
    let host = reviewing();
    project.write("lib.rs", &long_function("f"));
    let reply = send(&project, &host, edit(&project, "lib.rs"));
    assert!(context(&reply).contains("lib.rs:1 review"), "{reply}");
    let unchecked = send(&project, &host, stop(false));
    assert!(unchecked.get("decision").is_none());
    assert_eq!(message(&unchecked), text::UNCHECKED_TURN);
    project.write("lib.rs", &long_function("g"));
    assert_eq!(send(&project, &host, stop(false))["decision"], "block");
}

#[test]
fn a_turn_whose_start_git_pruned_starts_again_at_the_stop() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    let turns = project.0.join(".jevgate/turns");
    let file = std::fs::read_dir(&turns)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|e| e == "json"))
        .unwrap();
    let mut turn: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    turn["tree"] = json!("0".repeat(40));
    std::fs::write(&file, turn.to_string()).unwrap();
    project.write("lib.rs", &long_function("f"));
    let reply = send(&project, &host, stop(false));
    assert_eq!(message(&reply), text::UNCHECKED_TURN, "{reply}");
    project.write("lib.rs", &long_function("g"));
    assert_eq!(send(&project, &host, stop(false))["decision"], "block");
}

#[test]
fn a_session_start_records_a_turn_only_when_the_session_has_none() {
    let project = repository();
    let host = reviewing();
    let start = json!({"hook_event_name": "SessionStart", "source": "startup"});
    assert_eq!(send(&project, &host, start), json!({}));
    project.write("lib.rs", &long_function("f"));
    let compacted = json!({"hook_event_name": "SessionStart", "source": "compact"});
    assert_eq!(send(&project, &host, compacted), json!({}));
    assert_eq!(
        send(&project, &host, stop(false))["decision"],
        "block",
        "compacting mid-turn kept the turn's start"
    );
}

#[test]
fn a_provider_failure_never_blocks_and_is_told_to_the_person_and_the_agent() {
    let failures = [
        (
            Failing(|| {
                let failure = Failure {
                    status: 402,
                    ..Failure::default()
                };
                provider_error(&TYPESAFE, failure).into()
            }),
            "TypeSafe HTTP 402",
        ),
        (
            Failing(|| Unsent(&TYPESAFE).into()),
            "Cannot connect to TypeSafe",
        ),
    ];
    for (failing, said) in failures {
        let project = repository();
        let host = host(move || Box::new(failing));
        send(&project, &host, prompt("refactor"));
        project.write("lib.rs", &long_function("f"));
        let edited = send(&project, &host, edit(&project, "lib.rs"));
        assert!(edited.get("decision").is_none());
        assert!(
            context(&edited).starts_with(&format!("JevGate could not check lib.rs ({said}")),
            "{edited}"
        );
        assert!(context(&edited).ends_with("this is not a pass."));
        assert!(message(&edited).starts_with(&format!("JevGate could not check lib.rs: {said}")));
        let stopped = send(&project, &host, stop(false));
        assert_eq!(stopped.as_object().unwrap().len(), 1, "{stopped}");
        assert!(
            message(&stopped).starts_with(&format!("JevGate could not check this turn: {said}")),
            "{stopped}"
        );
        assert!(message(&stopped).ends_with("Nothing was blocked."));
        let next = send(&project, &host, prompt("go on"));
        assert!(
            context(&next).starts_with(&format!(
                "JevGate could not check the last turn's changes ({said}"
            )),
            "the agent hears of it at the next turn: {next}"
        );
        assert_eq!(context(&send(&project, &host, prompt("again"))), "", "once");
    }
}

#[test]
fn a_turn_whose_stop_could_not_be_checked_is_checked_with_the_next() {
    let project = repository();
    let unreachable = host(|| Box::new(Failing(|| Unsent(&TYPESAFE).into())));
    send(&project, &reviewing(), prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    let failed = send(&project, &unreachable, stop(false));
    assert!(failed.get("decision").is_none(), "{failed}");
    let next = send(&project, &reviewing(), prompt("add notes"));
    assert!(
        context(&next).ends_with("JevGate checks them with this turn's changes when it ends."),
        "{next}"
    );
    project.write("NOTES.md", "# Notes\n");
    let blocked = send(&project, &reviewing(), stop(false));
    assert!(
        blocked["reason"].as_str().unwrap_or_default().starts_with(
            "JevGate blocked the end of this turn (1 of at most 3): 1 finding in code changed since JevGate last checked fails the quality gate.\n- lib.rs:1 review "
        ),
        "the last turn's function is judged: {blocked}"
    );
    project.write("lib.rs", &function("f"));
    assert!(
        send(&project, &reviewing(), stop(true))
            .get("decision")
            .is_none()
    );
    assert_eq!(
        send(&project, &reviewing(), prompt("next")),
        json!({}),
        "a checked stop ends the carrying"
    );
}

#[test]
fn a_slow_provider_is_cut_at_the_budget() {
    let project = repository();
    let host = host(|| Box::new(Slow(Duration::from_secs(6))));
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    let started = Instant::now();
    let reply = send_within(&project, &host, stop(false), Duration::from_secs(2));
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the budget holds"
    );
    assert!(reply.get("decision").is_none());
    assert!(
        message(&reply).contains("the check did not finish within 2 s"),
        "{reply}"
    );
}

#[test]
fn a_session_lock_held_by_another_process_never_blocks() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    let held = crate::storage::Store::open(&project.0).unwrap();
    // Room for a slow runner to reach the lock before the reply is due.
    let reply = send_within(&project, &host, stop(false), Duration::from_secs(3));
    drop(held);
    assert!(reply.get("decision").is_none());
    assert!(message(&reply).contains("held its session lock"), "{reply}");
    assert_eq!(
        send(&project, &host, stop(false))["decision"],
        "block",
        "the turn kept its start, so the next stop checks it"
    );
}

#[test]
fn outside_git_nothing_is_checked_blocked_or_written() {
    let project = Project::new();
    project.write("lib.rs", &long_function("f"));
    let host = reviewing();
    for event in [prompt("go"), edit(&project, "lib.rs"), stop(false)] {
        let reply = send(&project, &host, event);
        assert!(reply.get("decision").is_none(), "{reply}");
        assert!(
            message(&reply).contains("is not in a Git repository"),
            "{reply}"
        );
    }
    assert!(!project.0.join(".jevgate").exists());
}

#[test]
fn an_invalid_configuration_never_blocks() {
    let project = repository();
    let host = reviewing();
    project.write("jevgate.toml", "rules = [\"no-such-rule\"]\n");
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    let reply = send(&project, &host, stop(false));
    assert!(reply.get("decision").is_none());
    assert!(message(&reply).contains("no-such-rule"), "{reply}");
}
