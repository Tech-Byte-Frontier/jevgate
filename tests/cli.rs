#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct Project(temp_dir::TempDir);
impl Project {
    fn new() -> Self {
        Self(temp_dir::TempDir::new("jevgate-cli"))
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_jevgate"));
        command
            .current_dir(&self.0)
            .env_remove("TYPESAFE_API_KEY")
            .env_remove("CI")
            .env("JEVGATE_CREDENTIAL_STORE", "file")
            .env("JEVGATE_CONFIG_DIR", self.0.join("isolated-auth"));
        command
    }
    /// Runs an offline preview: it succeeds, never prints the key and sends nothing.
    fn preview(&self, args: &[&str]) -> serde_json::Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("do-not-expose"));
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["api_requests"], 0);
        body
    }
    /// `auth status --offline --json` without a credential: exit 2 and an actionable error.
    fn unconfigured_status(&self, extra: &[&str], hint: &str) -> serde_json::Value {
        let output = self
            .command()
            .args(["auth", "status", "--offline", "--json"])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["configured"], false);
        assert!(body["error"].as_str().unwrap().contains(hint), "{body}");
        body
    }
    fn snapshot(&self) -> Option<serde_json::Value> {
        std::fs::read(self.0.join(".jevgate/latest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }
}

/// A function large enough to judge: five body lines.
const JUDGED_RS: &str = "fn f(values: &[i32]) -> i32 {\n    let mut total = 0;\n    for value in values {\n        total += value;\n    }\n    let doubled = total * 2;\n    doubled + 1\n}\n";

#[test]
fn removed_role_modes_are_rejected_without_state() {
    let project = Project::new();
    std::fs::write(project.0.join("example.py"), "def run():\n    return 1\n").unwrap();
    for flag in ["--roles-only", "--classification-cascade"] {
        let output = project
            .command()
            .args(["check", "example.py", flag, "--dry-run"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{flag}");
    }
    assert!(!project.0.join(".jevgate").exists());
}

#[test]
fn preview_sends_one_request_per_candidate_pair_without_local_metadata() {
    let project = Project::new();
    std::fs::write(project.0.join("mixed.py"), "def a(value):\n    name = value.strip().lower().replace(' ', '-')\n    record = dict(name=name, enabled=True, source='scheduled-import', owner=current_owner())\n    return save(record)\n\ndef b(value):\n    name = value.strip().lower().replace(' ', '-')\n    record = dict(name=name, enabled=True, source='scheduled-import', owner=current_owner())\n    return save(record)\n").unwrap();
    let output = project
        .command()
        .args([
            "check",
            "mixed.py",
            "--rule",
            "shared_logic",
            "--dry-run",
            "--show-requests",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 2);
    let requests = report["initial_requests"].as_array().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].get("jevgate").is_none());
    let state = &requests[0]["state"];
    assert_eq!(state["site_a"]["function"], "a");
    assert_eq!(state["site_b"]["function"], "b");
    assert!(requests[0]["questions"]["same"]["type"] == "score");
    let stage = &report["stages"]["duplicate-pair"];
    assert_eq!(stage["planned_requests"], 1);
    assert!(stage["planned_tokens"].as_u64().unwrap() > 0);
    assert!(!project.0.join(".jevgate").exists());
}

#[test]
fn default_preview_sends_units_without_automatic_context_or_state() {
    let project = Project::new();
    std::fs::write(
        project.0.join("lib.rs"),
        format!(
            "mod storage;\n{JUDGED_RS}{}",
            JUDGED_RS.replace("fn f(", "fn g(")
        ),
    )
    .unwrap();
    std::fs::write(project.0.join("storage.rs"), "pub fn save() {}").unwrap();
    std::fs::write(project.0.join(".env"), "TYPESAFE_API_KEY=do-not-expose").unwrap();
    let body = project.preview(&[
        "check",
        "lib.rs",
        "--dry-run",
        "--show-requests",
        "--format",
        "json",
    ]);
    assert_eq!(body["files"].as_array().unwrap().len(), 1);
    assert_eq!(body["files"][0]["context_files"], serde_json::json!([]));
    let stages = body["stages"].as_object().unwrap();
    assert_eq!(stages["functions"]["planned_requests"], 1);
    assert!(
        stages.get("outline").is_none(),
        "a short file is too small to split"
    );
    let functions = &body["initial_requests"][0];
    assert_eq!(functions["state"]["functions"].as_array().unwrap().len(), 2);
    assert!(functions["questions"]["f1_split"].is_object());
    assert!(!project.0.join(".jevgate").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn browser_report_is_local_and_does_not_change_json_or_failure_status() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    std::fs::write(project.0.join("api.py"), "def value(rows):\n    total = 0\n    for row in rows:\n        total += row\n    total *= 2\n    return total\n").unwrap();
    let bin = project.0.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let opener = bin.join("xdg-open");
    std::fs::write(
        &opener,
        "#!/bin/sh\nprintf '%s' \"$1\" > \"$REPORT_CAPTURE\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o700)).unwrap();
    let capture = project.0.join("opened.txt");
    let output = project
        .command()
        .env("PATH", &bin)
        .env("REPORT_CAPTURE", &capture)
        .args([
            "check",
            "api.py",
            "--cache-only",
            "--report",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["api_requests"], 0);
    assert_eq!(report["complete"], false);
    let html = std::fs::read_to_string(project.0.join(".jevgate/report.html")).unwrap();
    assert!(html.contains("api.py"));
    assert!(html.contains("\"status\":\"error\""));
    let deadline = Instant::now() + Duration::from_secs(3);
    // The opener creates the file before writing it; wait for its content.
    while std::fs::read_to_string(&capture).map_or(true, |text| text.is_empty())
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(&capture).unwrap(),
        project.0.join(".jevgate/report.html").to_str().unwrap()
    );
    // In CI the dashboard is written but no browser is started.
    std::fs::remove_file(&capture).unwrap();
    let ci = project
        .command()
        .env("PATH", &bin)
        .env("REPORT_CAPTURE", &capture)
        .env("CI", "true")
        .args(["check", "api.py", "--cache-only", "--report"])
        .output()
        .unwrap();
    assert_eq!(ci.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&ci.stderr).contains("report.html"));
    std::thread::sleep(Duration::from_millis(200));
    assert!(!capture.exists());
    let preview = project
        .command()
        .args(["check", "api.py", "--report", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(preview.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&preview.stderr).contains("cannot be used with"));
}

#[test]
fn initial_request_preview_is_explicit_offline_and_contains_selected_evidence() {
    let project = Project::new();
    let source = "def save(write):\n    try:\n        write()\n    except OSError:\n        log('retry')\n        return True\n\ndef submit(write):\n    return 'saved' if save(write) else 'failed'\n";
    std::fs::write(project.0.join("save.py"), source).unwrap();
    std::fs::write(project.0.join(".env"), "TYPESAFE_API_KEY=do-not-expose").unwrap();
    let invalid = project
        .command()
        .args(["check", "--show-requests"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    let report = project.preview(&[
        "check",
        "save.py",
        "--quick",
        "--rule",
        "function_simplification",
        "--dry-run",
        "--show-requests",
    ]);
    let requests = report["initial_requests"].as_array().unwrap();
    assert_eq!(requests.len(), 1);
    let functions = requests[0]["state"]["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 1, "submit is too small to judge");
    assert!(
        functions[0]["source"]
            .as_str()
            .unwrap()
            .contains("return True")
    );
    assert_eq!(requests[0]["questions"]["f0_split"]["type"], "score");
    assert!(!project.0.join(".jevgate").exists());
    let normal = project
        .command()
        .args([
            "check",
            "save.py",
            "--quick",
            "--dry-run",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&normal.stdout).unwrap();
    assert!(report.get("initial_requests").is_none());
}

/// Run Git in the project, with a fixed identity and no signing.
fn git(project: &Project, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(&project.0)
        .args([
            "-c",
            "user.name=JevGate test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

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

#[test]
fn absent_credentials_produce_operational_failure_with_atomic_report() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    let output = project
        .command()
        .args(["check", ".", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report = project.snapshot().unwrap();
    assert_eq!(report["complete"], false);
    assert_eq!(report["files"][0]["status"], "error");
    assert!(
        report["files"][0]["error"]
            .as_str()
            .unwrap()
            .contains("TYPESAFE_API_KEY")
    );
    assert!(project.0.join(".jevgate/.gitignore").exists());
}

#[test]
fn authentication_is_explicit_and_noninteractive_input_never_leaks() {
    use std::io::Write;
    let project = Project::new();
    let output = project.command().args(["auth", "login"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--with-key"));
    for input in ["private-first-key\nprivate-second-key", "private bad key"] {
        let mut child = project
            .command()
            .args(["auth", "login", "--with-key"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!text.contains("private"));
        assert!(!text.contains("Validating"));
    }
    assert!(!project.0.join("isolated-auth").exists());
}

#[test]
fn auth_status_explains_precedence_without_loading_review_config_or_showing_keys() {
    let project = Project::new();
    std::fs::write(project.0.join("jevgate.toml"), "deliberately invalid toml").unwrap();
    std::fs::write(project.0.join(".env"), "TYPESAFE_API_KEY=private-repo-key").unwrap();
    std::fs::write(
        project.0.join("selected.env"),
        "TYPESAFE_API_KEY=private-selected-key",
    )
    .unwrap();
    for (selected, environment, expected) in [
        (None, None, "repository .env:"),
        (Some("selected.env"), None, "--env-file:"),
        (
            Some("absent.env"),
            Some("private-environment-key"),
            "TYPESAFE_API_KEY environment variable",
        ),
    ] {
        let mut command = project.command();
        command.args(["auth", "status", "--offline", "--json"]);
        if let Some(path) = selected {
            command.args(["--env-file", path]);
        }
        if let Some(key) = environment {
            command.env("TYPESAFE_API_KEY", key);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-"));
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(body["source"].as_str().unwrap().starts_with(expected));
        assert_eq!(body["configured"], true);
        assert_eq!(body["connection_checked"], false);
        assert!(body["authenticated"].is_null());
    }
    project.unconfigured_status(&["--env-file", "missing.env"], "Selected --env-file");
}

#[cfg(unix)]
#[test]
fn saved_credential_is_shared_across_repositories() {
    let project = Project::new();
    save_credential(&project);
    for name in ["first-repository", "second-repository"] {
        let root = project.0.join(name);
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("jevgate.toml"), "").unwrap();
        let output = project
            .command()
            .current_dir(&root)
            .args(["auth", "status", "--offline", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-saved-key"));
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            body["source"]
                .as_str()
                .unwrap()
                .starts_with("protected file:")
        );
    }
}

#[cfg(unix)]
#[test]
fn logout_removes_the_saved_credential_and_names_remaining_overrides() {
    let project = Project::new();
    let saved = save_credential(&project);
    let local = "TYPESAFE_API_KEY=private-repo-key\nUNRELATED=keep\n";
    std::fs::write(project.0.join(".env"), local).unwrap();
    let output = project.command().args(["auth", "logout"]).output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Current override: repository .env:"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-"));
    assert!(!saved.exists());
    assert_eq!(
        std::fs::read_to_string(project.0.join(".env")).unwrap(),
        local
    );
    let output = project
        .command()
        .args(["auth", "logout"])
        .env("TYPESAFE_API_KEY", "private-environment-key")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Current override: TYPESAFE_API_KEY environment variable")
    );
}

/// A private saved credential in the project's isolated configuration directory.
#[cfg(unix)]
fn save_credential(project: &Project) -> std::path::PathBuf {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let config = project.0.join("isolated-auth");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&config)
        .unwrap();
    let saved = config.join("credentials");
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&saved)
        .unwrap()
        .write_all(b"private-saved-key")
        .unwrap();
    saved
}

#[test]
fn missing_saved_credential_has_machine_readable_actionable_status() {
    let project = Project::new();
    let body = project.unconfigured_status(&[], "jevgate auth login");
    assert!(body["authenticated"].is_null());
    assert!(!project.0.join(".jevgate").exists());
}

#[cfg(unix)]
#[test]
fn watcher_debounces_updates_and_releases_lock_after_credential_failure() {
    let project = Project::new();
    let source = project.0.join("lib.rs");
    std::fs::write(&source, JUDGED_RS).unwrap();
    let mut child = project
        .command()
        .args([
            "check",
            ".",
            "--watch",
            "--poll-ms",
            "50",
            "--debounce-ms",
            "50",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let wait_for = |predicate: &dyn Fn(&serde_json::Value) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(snapshot) = project.snapshot()
                && predicate(&snapshot)
            {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "watcher did not publish the expected snapshot"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait_for(&|r| r["settled"] == true);
    let second = project.command().args(["check", "."]).output().unwrap();
    assert_eq!(second.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&second.stderr).contains("Another JevGate session"));
    std::fs::write(&source, JUDGED_RS.replace("fn f(", "fn changed(")).unwrap();
    let updated = wait_for(&|r| r["generation"].as_u64().unwrap_or(0) > 1 && r["settled"] == true);
    assert_eq!(updated["api_requests"], 0);
    assert!(
        updated["files"][0]["error"]
            .as_str()
            .unwrap()
            .contains("TYPESAFE_API_KEY"),
        "{}",
        updated["files"][0]["error"]
    );
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(child.wait().unwrap().code(), Some(143));
    assert_eq!(
        project.snapshot().unwrap()["watcher_pid"],
        serde_json::Value::Null
    );
    let output = project
        .command()
        .args(["check", ".", "--format", "json"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Another JevGate session"));
}

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

/// The stages a dry-run preview plans.
fn stages(preview: &serde_json::Value) -> Vec<String> {
    preview["stages"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

fn dry_run(project: &Project, rules: &[&str]) -> serde_json::Value {
    let mut arguments = vec!["check", "--dry-run", "--format", "json"];
    arguments.extend_from_slice(rules);
    project.preview(&arguments)
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

#[test]
fn a_closed_output_pipe_ends_output_without_a_panic() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    for args in [
        &["rules"][..],
        &["check", "lib.rs", "--dry-run", "--format", "json"],
    ] {
        let mut child = project
            .command()
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Close the reading end before the command writes, as `| head` does.
        drop(child.stdout.take());
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("panicked"), "{args:?}: {stderr}");
        assert_eq!(output.status.code(), Some(0), "{args:?}: {stderr}");
    }
}
