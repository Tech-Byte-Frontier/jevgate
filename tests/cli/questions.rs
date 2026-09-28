//! Custom questions: their files, listing, validation and planning.
use super::*;

const BODY_LOGS: &str = "question = \"Does this function write a request body to a log?\"\nunit = \"function\"\nlevel = \"consider\"\n";

/// A project whose `.jevgate/questions/` holds `files`, each an id and its text.
fn with_questions(files: &[(&str, &str)]) -> Project {
    let project = Project::new();
    std::fs::create_dir_all(project.0.join(".jevgate/questions")).unwrap();
    for (id, text) in files {
        std::fs::write(
            project.0.join(format!(".jevgate/questions/{id}.toml")),
            text,
        )
        .unwrap();
    }
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    project
}

#[test]
fn rules_lists_the_questions_of_the_questions_directory() {
    let project = with_questions(&[("body-logs", BODY_LOGS)]);
    let output = project.command().arg("rules").output().unwrap();
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(
        table.contains("custom/body-logs") && table.contains("[function, consider at 0.80]"),
        "{table}"
    );
    assert!(table.contains("documentation, custom, default"), "{table}");
    let output = project
        .command()
        .args(["rules", "--format", "json"])
        .output()
        .unwrap();
    let rules: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let custom = rules.as_array().unwrap().last().unwrap();
    assert_eq!(custom["id"], "custom/body-logs");
    assert_eq!(custom["group"], "custom");
    assert_eq!(custom["custom"]["level"], "consider");
    assert_eq!(
        custom["custom"]["source"],
        ".jevgate/questions/body-logs.toml"
    );
}

#[test]
fn an_invalid_question_stops_every_command_that_reads_the_configuration() {
    let project = with_questions(&[("Body Logs", BODY_LOGS)]);
    for args in [&["rules"][..], &["check", "--dry-run"]] {
        let output = project.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error
                .contains("Invalid question id \"Body Logs\" in .jevgate/questions/Body Logs.toml"),
            "{error}"
        );
    }
}

#[test]
fn a_dry_run_plans_custom_questions_and_a_reviewed_config_leaves_the_directory_out() {
    let project = with_questions(&[("body-logs", BODY_LOGS)]);
    let report = dry_run(&project, &["--rule", "custom"]);
    assert_eq!(stages(&report), ["custom"]);
    assert_eq!(report["rules"], serde_json::json!(["custom/body-logs"]));
    assert_eq!(report["stages"]["custom"]["planned_requests"], 1);
    assert_eq!(
        report["fail_on_mature"]["custom/body-logs"],
        serde_json::json!(["consider"]),
        "the default gate fails a question at its own level"
    );
    std::fs::write(project.0.join("policy.toml"), "").unwrap();
    let reviewed = dry_run(&project, &["--config", "policy.toml"]);
    assert!(
        !reviewed["rules"].to_string().contains("custom/"),
        "--config reads only its own [[question]] tables"
    );
    let output = project
        .command()
        .args(["check", "--dry-run", "--config", "policy.toml"])
        .output()
        .unwrap();
    let note = String::from_utf8_lossy(&output.stderr);
    assert!(
        note.contains(
            "--config leaves the question files of .jevgate/questions/ unread (custom/body-logs)"
        ),
        "{note}"
    );
    let copy = project.0.join("reviewed-questions");
    std::fs::create_dir(&copy).unwrap();
    std::fs::write(copy.join("body-logs.toml"), BODY_LOGS).unwrap();
    let pinned = dry_run(
        &project,
        &[
            "--config",
            "policy.toml",
            "--questions",
            "reviewed-questions",
        ],
    );
    assert!(
        pinned["rules"].to_string().contains("custom/body-logs"),
        "--questions reads a reviewed copy: {}",
        pinned["rules"]
    );
    let output = project
        .command()
        .args(["check", "--dry-run", "--questions", "missing"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("No questions directory missing"));
    let output = project
        .command()
        .args([
            "check",
            "--dry-run",
            "--config",
            "policy.toml",
            "--rule",
            "custom",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("none is defined"));
}

#[test]
fn a_hunk_question_without_base_says_it_was_not_asked() {
    let project = with_questions(&[(
        "no-unwrap",
        "question = \"Does this change add an unwrap?\"\nunit = \"hunk\"\n",
    )]);
    let output = project
        .command()
        .args(["check", "--dry-run", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("custom/no-unwrap was not asked: it needs --base"),
        "{error}"
    );
}

/// A question file with a failing and a passing example.
const EXAMPLED: &str = "question = \"Does this function write a request body to a log?\"\nunit = \"function\"\n\n[[failing]]\npath = \"src/orders.rs\"\ncode = \"fn charge(req: &Request) {\\n    log(req.body());\\n}\\n\"\n\n[[passing]]\npath = \"src/orders.rs\"\ncode = \"fn charge(req: &Request) {\\n    log(req.id());\\n}\\n\"\n";

#[test]
fn rules_test_asks_examples_and_a_dry_run_checks_them_offline() {
    let project = with_questions(&[("body-logs", EXAMPLED), ("other", BODY_LOGS)]);
    let output = project
        .command()
        .args(["rules", "test", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.starts_with("JevGate: rules test dry run · 2 examples · 1 question · 2 requests, 0 answered by the cache · ~"),
        "{text}"
    );
    assert!(
        text.ends_with("Without examples: custom/other.\n"),
        "{text}"
    );
    assert!(!project.0.join(".jevgate/cache").exists());
    let output = project.command().args(["rules", "test"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "no key: incomplete");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.starts_with("JevGate: rules test · incomplete: 2 of 2 examples not answered · ")
            && text.contains("No API key configured"),
        "{text}"
    );
    let output = project
        .command()
        .args(["rules", "--format", "json"])
        .output()
        .unwrap();
    let rules: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let described = rules
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "custom/body-logs")
        .unwrap();
    assert_eq!(
        described["custom"]["examples"],
        serde_json::json!({"failing": 1, "passing": 1})
    );
}

#[test]
fn an_invalid_example_stops_every_command_and_an_unaskable_one_the_test() {
    let unpathed = EXAMPLED.replacen("path = \"src/orders.rs\"\n", "", 1);
    let project = with_questions(&[("body-logs", &unpathed)]);
    for args in [&["rules"][..], &["check", "--dry-run"], &["rules", "test"]] {
        let output = project.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("Question custom/body-logs in .jevgate/questions/body-logs.toml: failing example 1: `path` is required with `code`"),
            "{error}"
        );
    }
    let fieldless = EXAMPLED.replace(
        "fn charge(req: &Request) {\\n    log(req.id());\\n}\\n",
        "const LIMIT: u32 = 3;\\n",
    );
    let project = with_questions(&[("body-logs", &fieldless)]);
    let output = project
        .command()
        .args(["rules", "test", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("  error  passing 1            src/orders.rs: it holds no function"),
        "{text}"
    );
    let output = project
        .command()
        .args(["check", "--dry-run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "an example that holds no unit never stops a check"
    );
}

#[test]
fn a_question_the_rules_list_leaves_out_is_named_as_unasked() {
    let project = with_questions(&[("body-logs", BODY_LOGS)]);
    let stderr = |project: &Project, args: &[&str]| {
        let output = project.command().args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}");
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    let note = "custom/body-logs is not asked: the `rules` list in jevgate.toml leaves it out; add \"custom\" to it";
    std::fs::write(
        project.0.join("jevgate.toml"),
        "rules = [\"maintainability\"]\n",
    )
    .unwrap();
    assert!(stderr(&project, &["check", "--dry-run"]).contains(note));
    let named = ["check", "--dry-run", "--rule", "maintainability"];
    assert!(
        !stderr(&project, &named).contains(note),
        "chosen on the command line"
    );
    let skipped = ["check", "--dry-run", "--skip-rule", "custom/body-logs"];
    assert!(
        !stderr(&project, &skipped).contains(note),
        "left out on purpose"
    );
    let listed = "rules = [\"maintainability\", \"custom\"]\n";
    std::fs::write(project.0.join("jevgate.toml"), listed).unwrap();
    assert!(!stderr(&project, &["check", "--dry-run"]).contains(note));
    std::fs::write(project.0.join("jevgate.toml"), "rules = [\"security\"]\n").unwrap();
    let added = stderr(&project, &["rules", "add", "n-plus-one"]);
    assert!(added.contains("custom/n-plus-one is not asked"), "{added}");
}
