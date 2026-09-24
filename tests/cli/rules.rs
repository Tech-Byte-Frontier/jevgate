//! Rule selection, levels, the rules table and `init`.
use super::*;

#[test]
fn catalog_and_cli_expose_only_the_supported_maintainability_checks() {
    let project = Project::new();
    let output = project
        .command()
        .args(["rules", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let rules: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let keys: Vec<_> = rules
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "file_organization",
            "function_simplification",
            "shared_logic",
            "hardcoded_values",
            "injection",
            "sensitive_data",
            "unsafe_settings",
            "access_control",
            "workflows",
            "test_value",
            "test_redundancy",
            "agent_context",
            "large_docs",
            "doc_staleness",
            "doc_duplication"
        ]
    );
    for arguments in [
        vec!["check", "--rule", "contracts", "--dry-run"],
        vec!["record"],
        vec!["check", "--evidence", "old.json"],
    ] {
        let output = project.command().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!project.0.join(".jevgate").exists());
    }
}

#[test]
fn rules_table_names_rules_groups_and_opt_in_rules() {
    let project = Project::new();
    let output = project.command().arg("rules").output().unwrap();
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(table.starts_with("RULE") && table.contains("maintainability/shared-logic"));
    assert!(
        table.contains("Groups: maintainability, security, tests, documentation"),
        "{table}"
    );
    assert!(table.contains("security/injection") && table.contains("opt-in"));
}

#[test]
fn security_runs_only_when_selected() {
    let project = Project::new();
    std::fs::write(
        project.0.join("store.rs"),
        "fn load(conn: &Connection, table: &str) {\n    conn.execute(&format!(\"DELETE FROM {table}\"), []).unwrap();\n}\n",
    )
    .unwrap();
    assert!(!stages(&dry_run(&project, &[])).contains(&"security".to_string()));
    assert_eq!(
        stages(&dry_run(&project, &["--rule", "security"])),
        ["security"]
    );
}

#[test]
fn documentation_runs_only_when_selected_and_reports_what_harnesses_load() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    std::fs::write(
        project.0.join("AGENTS.md"),
        "# Build\nRun `cargo test` before a commit.\n",
    )
    .unwrap();
    assert!(!stages(&dry_run(&project, &[])).contains(&"instructions".to_string()));
    let documentation = dry_run(&project, &["--rule", "documentation"]);
    assert_eq!(stages(&documentation), ["instructions"]);
    let codex = &documentation["context_load"]["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["harness"] == "Codex")
        .unwrap()["files"];
    assert_eq!(codex, &serde_json::json!(["AGENTS.md"]));
}

#[test]
fn skipped_rules_and_rule_levels_shape_the_run() {
    let project = Project::new();
    std::fs::write(
        project.0.join("lib.rs"),
        JUDGED_RS.replace("total * 2", "total * 86400"),
    )
    .unwrap();
    assert!(stages(&dry_run(&project, &[])).contains(&"values".to_string()));
    let preview = dry_run(
        &project,
        &[
            "--rule",
            "maintainability",
            "--skip-rule",
            "hardcoded_values",
            "--fail-on",
            "maintainability/shared-logic=consider",
        ],
    );
    assert_eq!(stages(&preview), ["functions"]);
    assert_eq!(preview["fail_on"], serde_json::json!(["review"]));
    assert_eq!(
        preview["fail_on_rules"],
        serde_json::json!({"maintainability/shared-logic": ["consider"]})
    );
}

#[test]
fn unknown_rules_and_levels_are_rejected() {
    let project = Project::new();
    for arguments in [
        vec!["check", "--rule", "securty", "--dry-run"],
        vec!["check", "--fail-on", "tests=sometimes", "--dry-run"],
    ] {
        let output = project.command().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
}

#[test]
fn init_writes_a_valid_configuration_once() {
    let project = Project::new();
    std::fs::create_dir_all(project.0.join("src")).unwrap();
    std::fs::write(project.0.join("src/lib.rs"), JUDGED_RS).unwrap();
    let output = project.command().arg("init").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("src/**"));
    let config = std::fs::read_to_string(project.0.join("jevgate.toml")).unwrap();
    assert!(config.contains("upload_allow = [\"src/**\"]"), "{config}");
    project.preview(&["check", "--dry-run", "--format", "json"]);
    let again = project.command().arg("init").output().unwrap();
    assert_eq!(again.status.code(), Some(2), "an existing file is kept");
    let forced = project
        .command()
        .args(["init", "--force"])
        .output()
        .unwrap();
    assert!(forced.status.success());
}
