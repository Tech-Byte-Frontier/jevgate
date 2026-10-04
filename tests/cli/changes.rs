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
    let project = Project::committed();
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
fn base_judges_changed_lines_unless_whole_files_is_given() {
    let project = Project::new();
    let source = format!("{JUDGED_RS}{}", JUDGED_RS.replace("fn f(", "fn g("));
    std::fs::write(project.0.join("lib.rs"), &source).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "lib.rs"]);
    git(&project, &["commit", "-qm", "baseline"]);
    std::fs::write(project.0.join("lib.rs"), source.replacen("+ 1", "+ 2", 1)).unwrap();
    // `f` and `g` share a pack; the change touched `f` alone.
    let functions = |flags: &[&str], environment: &[(&str, &str)]| {
        let output = project
            .command()
            .args(["check", "--dry-run", "--format", "json"])
            .args(["--base", "HEAD", "--show-requests"])
            .args(flags)
            .envs(environment.iter().copied())
            .output()
            .unwrap();
        assert!(output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let names: Vec<String> = report["initial_requests"][0]["state"]["functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect();
        (report["scope"].clone(), names)
    };
    let changed = (serde_json::json!("changed-lines"), vec!["f".to_string()]);
    assert_eq!(functions(&[], &[]), changed);
    assert_eq!(
        functions(&[], &[("GIT_DIFF_OPTS", "--unified=6")]),
        changed,
        "Git lets GIT_DIFF_OPTS outrank -U0, which would count context lines as changed"
    );
    assert_eq!(
        functions(&["--whole-files"], &[]),
        (
            serde_json::json!("whole-files"),
            vec!["f".to_string(), "g".to_string()]
        )
    );
    let usage = project
        .command()
        .args(["check", "--whole-files", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&usage.stderr).contains("--base"));
}

#[test]
fn base_reads_the_repository_git_dir_names() {
    // A work tree whose repository is kept elsewhere is found only through
    // GIT_DIR and GIT_WORK_TREE: a check honors them, as it honors those Git
    // sets for a hook, where the tests' own Git drops them.
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    std::fs::write(project.0.join("other.rs"), JUDGED_RS).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "baseline"]);
    let elsewhere = Project::new();
    let store = elsewhere.0.join("store.git");
    std::fs::rename(project.0.join(".git"), &store).unwrap();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS.replace("+ 1", "+ 2")).unwrap();
    let output = project
        .command()
        .env("GIT_DIR", plain(&store))
        .env("GIT_WORK_TREE", plain(&project.0))
        .args(["check", "--dry-run", "--format", "json", "--base", "HEAD"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let paths: Vec<&str> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["lib.rs"]);
}

/// A path as a person would give it to Git. The tests' temporary directories
/// are canonical, which on Windows is the verbatim form `\\?\C:\…`, and Git for
/// Windows reads a `GIT_DIR` in that form as "not a git repository".
pub(super) fn plain(path: &std::path::Path) -> std::path::PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"UNC\") => std::path::PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
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
    let injection = "f0_interpreted".to_string();
    assert!(asked(&project, &[]).contains(&injection));
    let ci = ["--config", "policy/ci.toml"];
    assert!(!asked(&project, &ci).contains(&injection));
    let reviewed = dry_run(&project, &ci);
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

/// A loader repeated in each file, named `name`, reading `field`.
fn loader(name: &str, field: &str, default: &str) -> String {
    format!(
        "fn {name}(path: &str) -> Result<User> {{\n    let text = std::fs::read_to_string(path)?;\n    let value: Value = serde_json::from_str(&text)?;\n    let name = value[\"{field}\"].as_str().unwrap_or(\"{default}\").trim().to_string();\n    Ok(User {{ name }})\n}}\n"
    )
}

#[test]
fn base_points_a_repeat_at_the_copy_the_change_touched() {
    use mock_provider::{MockProvider, Reply, answer};
    let provider = MockProvider::start(|received| Reply::json(200, &answer(&received.json(), 2)));
    let tail = "\nfn tail(values: &[i32]) -> i32 {\n    let mut total = 0;\n    for value in values {\n        total += value;\n    }\n    total + 1\n}\n";
    let project = Project::committed_with(&[
        (
            "a.rs",
            &format!("{}{tail}", loader("load_user", "name", "anonymous")),
        ),
        ("b.rs", &loader("load_team", "title", "anonymous")),
    ]);
    let check = |project: &Project| -> serde_json::Value {
        let output = project
            .asking(&provider, "key")
            .args(["check", "--base", "HEAD", "--format", "json"])
            .args(["--rule", "maintainability/shared-logic"])
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
    };
    let repeats = |report: &serde_json::Value| -> Vec<(String, serde_json::Value)> {
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|file| {
                file["findings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(move |f| (file["path"].as_str().unwrap().to_string(), f.clone()))
            })
            .collect()
    };
    // The change edits `a.rs` beside its copy and `b.rs`'s copy.
    let write = |a_default: &str, b_default: &str, tail_end: &str| {
        let a = format!(
            "{}{}",
            loader("load_user", "name", a_default),
            tail.replace("total + 1", tail_end)
        );
        std::fs::write(project.0.join("a.rs"), a).unwrap();
        std::fs::write(
            project.0.join("b.rs"),
            loader("load_team", "title", b_default),
        )
        .unwrap();
    };
    write("anonymous", "nobody", "total + 2");
    let found = repeats(&check(&project));
    assert_eq!(found.len(), 1, "{found:?}");
    let (path, finding) = &found[0];
    assert_eq!(
        (path.as_str(), &finding["line"]),
        ("b.rs", &serde_json::json!(2))
    );
    assert_eq!(
        finding["untouched"],
        serde_json::json!([{"path": "a.rs", "start_line": 2, "end_line": 5, "symbol": "load_user", "file_changed": true}])
    );
    // A change to both copies keeps the repeat as it was found.
    write("nobody", "nobody", "total + 1");
    let found = repeats(&check(&project));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "a.rs");
    assert!(found[0].1.get("untouched").is_none(), "{:?}", found[0].1);
}
