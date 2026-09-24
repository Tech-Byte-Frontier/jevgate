//! `--base` change scopes, `--config` and the GitHub format.
use super::*;

#[test]
fn base_errors_and_empty_changes_are_distinct_cli_outcomes() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), "fn run() {}\n").unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "lib.rs"]);
    git(&project, &["commit", "-qm", "baseline"]);
    let empty = project
        .command()
        .args(["check", "--base", "HEAD", "--quick", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        empty.status.success(),
        "{}",
        String::from_utf8_lossy(&empty.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&empty.stdout).unwrap();
    assert_eq!(report["status"], "no-changed-source");
    assert_eq!(report["api_requests"], 0);
    let invalid = project
        .command()
        .args(["check", "--base", "no-such-ref", "--quick"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("Git"));
}

#[test]
fn base_reviews_changes_since_the_fork_point_like_a_pull_request() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "lib.rs"]);
    git(&project, &["commit", "-qm", "start"]);
    git(&project, &["tag", "start"]);
    git(&project, &["checkout", "-qb", "feature"]);
    std::fs::write(project.0.join("feature.rs"), JUDGED_RS).unwrap();
    git(&project, &["add", "feature.rs"]);
    git(&project, &["commit", "-qm", "feature"]);
    // The base branch moves on after the fork: its changes are not the feature's.
    git(&project, &["checkout", "-qb", "main-later", "start"]);
    std::fs::write(
        project.0.join("lib.rs"),
        JUDGED_RS.replace("fn f(", "fn g("),
    )
    .unwrap();
    std::fs::write(project.0.join("later.rs"), JUDGED_RS).unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "later"]);
    git(&project, &["checkout", "-q", "feature"]);
    let report = dry_run(&project, &["--base", "main-later"]);
    let paths: Vec<&str> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["feature.rs"]);
    assert_eq!(report["deleted_files"], serde_json::json!([]));
}

#[test]
fn a_config_file_given_by_path_replaces_the_repository_one() {
    let project = Project::new();
    std::fs::write(
        project.0.join("store.rs"),
        "fn load(conn: &Connection, table: &str) {\n    conn.execute(&format!(\"DELETE FROM {table}\"), []).unwrap();\n}\n",
    )
    .unwrap();
    std::fs::write(project.0.join("jevgate.toml"), "rules = [\"security\"]\n").unwrap();
    std::fs::create_dir_all(project.0.join("policy")).unwrap();
    std::fs::write(project.0.join("policy/ci.toml"), "fail_on = [\"none\"]\n").unwrap();
    assert_eq!(stages(&dry_run(&project, &[])), ["security"]);
    let reviewed = dry_run(&project, &["--config", "policy/ci.toml"]);
    assert!(!stages(&reviewed).contains(&"security".to_string()));
    assert_eq!(reviewed["fail_on"], serde_json::json!(["none"]));
    let missing = project
        .command()
        .args(["check", "--dry-run", "--config", "policy/absent.toml"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("absent.toml"));
}

#[test]
fn github_format_writes_a_job_summary_and_the_agent_text() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    let summary = project.0.join("summary.md");
    let output = project
        .command()
        .env("GITHUB_STEP_SUMMARY", &summary)
        .args(["check", "--dry-run", "--format", "github"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("JevGate: "));
    let text = std::fs::read_to_string(summary).unwrap();
    assert!(text.starts_with("### JevGate: "), "{text}");
}
