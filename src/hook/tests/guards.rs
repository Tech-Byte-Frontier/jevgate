//! Gaming the gate within a turn: an allow comment, a baseline or a looser
//! jevgate.toml does not let the agent stop, and what a turn does to the
//! checks around the code reaches the agent after the edit and the person
//! at the stop.
use super::*;

/// Every answer at the bottom of its scale: the code is clear.
fn clearing() -> Host {
    host(|| Box::new(Mock::default()))
}

/// The repository with a committed test, `adds`, and a turn begun by `host`
/// asking to make it pass.
fn tested_turn(host: &Host) -> Project {
    let project = repository();
    project.write(
        "tests.rs",
        "#[test]\nfn adds() {\n    assert_eq!(1 + 1, 2);\n}\n",
    );
    project.git(&["add", "."]);
    project.git(&["commit", "-qm", "tests"]);
    send(&project, host, prompt("make it pass"));
    project
}

/// The block reason of `reply`, or the reply itself when it did not block.
fn reason(reply: &Value) -> String {
    reply["reason"]
        .as_str()
        .map_or_else(|| reply.to_string(), str::to_string)
}

#[test]
fn an_allow_comment_added_in_the_turn_does_not_let_the_agent_stop() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    let allowed = format!(
        "// jevgate: allow(function-simplification) it reads as one job\n{}",
        long_function("f")
    );
    project.write("lib.rs", &allowed);
    let blocked = send(&project, &host, stop(false));
    assert_eq!(blocked["decision"], "block", "{blocked}");
    let why = reason(&blocked);
    assert!(
        why.contains("- lib.rs:2 review maintainability/function-simplification (fails the gate; accepted this turn): "),
        "{why}"
    );
    assert!(
        why.ends_with("accepting findings is the person's call, so leave that to them."),
        "{why}"
    );
    assert!(
        message(&blocked).ends_with("JevGate: this turn adds 1 `jevgate: allow` comment (lib.rs:1 accepts a finding: // jevgate: allow(function-simplification) it reads as one job). Its gate read jevgate.toml, the baseline and `jevgate: allow` comments as they were when the turn began."),
        "{blocked}"
    );
    // From the next turn on, the comment is the person's to keep or remove.
    send(&project, &host, prompt("go on"));
    project.write("lib.rs", &format!("{allowed}// done\n"));
    assert_eq!(send(&project, &host, stop(false)), json!({}));
}

#[test]
fn a_baseline_written_in_the_turn_does_not_let_the_agent_stop() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    project.write("lib.rs", &long_function("f"));
    assert_eq!(send(&project, &host, stop(false))["decision"], "block");
    crate::baseline::write(&project.0, true, None).unwrap();
    let again = send(&project, &host, stop(true));
    assert!(reason(&again).contains("(2 of at most 3)"), "{again}");
    assert!(
        reason(&again).contains("(fails the gate; accepted this turn)"),
        "{again}"
    );
    assert!(
        message(&again)
            .contains("this turn edits jevgate-baseline.json (jevgate-baseline.json is added)"),
        "{again}"
    );
    let kept = send(&project, &host, stop(true));
    assert!(
        kept.get("decision").is_none(),
        "nothing changed since: {kept}"
    );
}

#[test]
fn a_looser_configuration_in_the_turn_does_not_let_the_agent_stop() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("refactor"));
    project.write("jevgate.toml", "rules = [\"comments\"]\n");
    project.write("lib.rs", &long_function("f"));
    let edited = send(&project, &host, edit(&project, "lib.rs"));
    assert!(
        context(&edited)
            .contains("lib.rs:1 review maintainability/function-simplification (fails the gate)"),
        "the edit is checked with the turn's configuration too: {edited}"
    );
    let blocked = send(&project, &host, stop(false));
    assert_eq!(blocked["decision"], "block", "{blocked}");
    assert!(
        message(&blocked).contains("this turn edits jevgate.toml (jevgate.toml is edited: rules)"),
        "{blocked}"
    );
    // The next turn starts with the new configuration.
    send(&project, &host, prompt("go on"));
    project.write("lib.rs", &long_function("g"));
    assert_eq!(send(&project, &host, stop(false)), json!({}));
}

#[test]
fn an_edit_to_jevgate_toml_alone_is_told_to_the_person() {
    let project = repository();
    let host = reviewing();
    send(&project, &host, prompt("tighten the gate"));
    project.write(
        "jevgate.toml",
        "rules = [\"function-simplification\"]\nfail_on = [\"consider\"]\n",
    );
    let noticed = send(&project, &host, edit(&project, "jevgate.toml"));
    assert!(
        context(&noticed).starts_with("JevGate noticed that this turn edits jevgate.toml so far:\n- jevgate.toml is edited: fail_on\n"),
        "{noticed}"
    );
    let stopped = send(&project, &host, stop(false));
    assert!(stopped.get("decision").is_none());
    assert_eq!(
        message(&stopped),
        "JevGate: this turn edits jevgate.toml (jevgate.toml is edited: fail_on). Its gate read jevgate.toml, the baseline and `jevgate: allow` comments as they were when the turn began."
    );
}

#[test]
fn a_suppression_and_a_skipped_test_reach_the_agent_once_and_the_person_at_the_stop() {
    let host = clearing();
    let project = tested_turn(&host);
    project.write("lib.rs", &format!("#[allow(dead_code)]\n{}", function("f")));
    project.write(
        "tests.rs",
        "#[test]\n#[ignore = \"flaky\"]\nfn adds() {\n    assert_eq!(1 + 1, 2);\n}\n",
    );
    let first = send(&project, &host, edit(&project, "lib.rs"));
    assert_eq!(
        context(&first),
        "JevGate noticed that this turn adds 1 suppression so far:\n- lib.rs:1 turns off the Rust compiler or Clippy here: #[allow(dead_code)]\nJevGate reports these to the person at the end of the turn. Within a turn it reads jevgate.toml, the baseline and `jevgate: allow` comments as they were when the turn began."
    );
    assert!(
        send(&project, &host, edit(&project, "lib.rs"))
            .get("hookSpecificOutput")
            .is_none(),
        "once a turn"
    );
    let tests = send(&project, &host, edit(&project, "tests.rs"));
    assert!(
        context(&tests).contains("- tests.rs:2 skips a test: #[ignore = \"flaky\"]"),
        "{tests}"
    );
    let stopped = send(&project, &host, stop(false));
    assert!(stopped.get("decision").is_none(), "{stopped}");
    assert_eq!(
        message(&stopped),
        "JevGate: this turn adds 1 suppression and skips 1 test (lib.rs:1 turns off the Rust compiler or Clippy here: #[allow(dead_code)]; tests.rs:2 skips a test: #[ignore = \"flaky\"])."
    );
}

#[test]
fn a_test_that_checks_less_is_told_to_the_person() {
    let host = reviewing();
    let project = tested_turn(&host);
    project.write(
        "tests.rs",
        "#[test]\nfn adds() {\n    assert!(1 + 1 > 0);\n}\n",
    );
    let stopped = send(&project, &host, stop(false));
    assert!(stopped.get("decision").is_none(), "{stopped}");
    assert_eq!(
        message(&stopped),
        "JevGate: this turn weakens 1 test (tests.rs:2 `adds` checks less than before (95%))."
    );
}
