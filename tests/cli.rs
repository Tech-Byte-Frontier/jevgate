use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "jevgate-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_jevgate"));
        command
            .current_dir(&self.0)
            .env_remove("TYPESAFE_API_KEY")
            .env("JEVGATE_CREDENTIAL_STORE", "file")
            .env("JEVGATE_CONFIG_DIR", self.0.join("isolated-auth"));
        command
    }
    fn snapshot(&self) -> Option<serde_json::Value> {
        std::fs::read(self.0.join(".jevgate/latest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn roles_only_preview_excludes_verdicts_and_conflicting_modes() {
    let project = Project::new();
    std::fs::write(
        project.0.join("example.py"),
        "def test_connection(address):\n    return address.strip().lower()\n",
    )
    .unwrap();
    let output = project
        .command()
        .args([
            "check",
            "example.py",
            "--roles-only",
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
    assert_eq!(report["command"], "classify-roles");
    assert_eq!(report["stages"]["roles"]["planned_requests"], 1);
    let request = &report["initial_requests"][0];
    assert!(
        request["questions"]
            .as_object()
            .unwrap()
            .keys()
            .all(|key| key.starts_with("role_"))
    );
    assert!(request["state"]["file"].get("role").is_none());
    for flag in ["--classification-cascade", "--report"] {
        assert!(
            !project
                .command()
                .args(["check", "example.py", "--roles-only", flag])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    assert!(!project.0.join(".jevgate").exists());
}

#[test]
fn cascade_preview_is_opt_in_bounded_and_shares_one_request() {
    let project = Project::new();
    std::fs::write(project.0.join("mixed.py"), "def a(value):\n    name = value.strip().lower()\n    record = dict(name=name, enabled=True)\n    return save(record)\n\ndef b(value):\n    name = value.strip().lower()\n    record = dict(name=name, enabled=True)\n    return save(record)\n").unwrap();
    let preview = |enabled: bool| {
        let mut command = project.command();
        command.args([
            "check",
            "mixed.py",
            "--rule",
            "shared_logic",
            "--dry-run",
            "--show-requests",
        ]);
        if enabled {
            command.arg("--classification-cascade");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let baseline = preview(false);
    let experiment = preview(true);
    assert_eq!(experiment["initial_requests"].as_array().unwrap().len(), 1);
    assert_eq!(
        baseline["initial_requests"][0]["state"],
        experiment["initial_requests"][0]["state"]
    );
    let questions = experiment["initial_requests"][0]["questions"]
        .as_object()
        .unwrap();
    assert!(questions.contains_key("cascade_specialist_0"));
    assert!(
        questions
            .keys()
            .filter(|k| k.starts_with("cascade_"))
            .count()
            <= 37
    );
    assert!(
        !baseline["initial_requests"][0]["questions"]
            .as_object()
            .unwrap()
            .contains_key("cascade_file_tests")
    );
    assert!(!project.0.join(".jevgate").exists());
}

#[test]
fn default_preview_batches_maintainability_without_automatic_context_or_state() {
    let project = Project::new();
    std::fs::write(
        project.0.join("lib.rs"),
        "mod storage; fn f() { storage::save(); }",
    )
    .unwrap();
    std::fs::write(project.0.join("storage.rs"), "pub fn save() {}").unwrap();
    std::fs::write(project.0.join(".env"), "TYPESAFE_API_KEY=do-not-expose").unwrap();
    let output = project
        .command()
        .args([
            "check",
            "lib.rs",
            "--dry-run",
            "--show-requests",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("do-not-expose"));
    let body: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["api_requests"], 0);
    assert_eq!(body["files"].as_array().unwrap().len(), 1);
    assert_eq!(body["files"][0]["context_files"], serde_json::json!([]));
    assert_eq!(body["initial_requests"].as_array().unwrap().len(), 1);
    let questions = body["initial_requests"][0]["questions"]
        .as_object()
        .unwrap();
    assert_eq!(questions.len(), 7);
    assert!(
        questions.contains_key("operation_probe_1") || questions.contains_key("operation_probe_0")
    );
    assert!(questions.contains_key("file_organization"));
    assert!(questions.contains_key("function_simplification"));
    assert!(questions.contains_key("shared_logic"));
    assert!(!project.0.join(".jevgate").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn browser_report_is_local_and_does_not_change_json_or_failure_status() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    std::fs::write(project.0.join("api.py"), "def value():\n    return 1\n").unwrap();
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
    while !capture.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(&capture).unwrap(),
        project.0.join(".jevgate/report.html").to_str().unwrap()
    );
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
    let source = "def save(write):\n    try:\n        write()\n    except OSError:\n        return True\n\ndef submit(write):\n    return 'saved' if save(write) else 'failed'\n";
    std::fs::write(project.0.join("save.py"), source).unwrap();
    std::fs::write(project.0.join(".env"), "TYPESAFE_API_KEY=do-not-expose").unwrap();
    let invalid = project
        .command()
        .args(["check", "--show-requests"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    let output = project
        .command()
        .args([
            "check",
            "save.py",
            "--quick",
            "--rule",
            "function_simplification",
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
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("do-not-expose"));
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["api_requests"], 0);
    let requests = report["initial_requests"].as_array().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]["state"]["file"]["source"]
            .as_str()
            .unwrap()
            .contains("return True")
    );
    assert!(
        requests[0]["state"]["file"]["source"]
            .to_string()
            .contains("saved")
    );
    assert_eq!(
        requests[0]["questions"]["function_simplification"]["type"],
        "choice"
    );
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

#[test]
fn base_errors_and_empty_changes_are_distinct_cli_outcomes() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), "fn run() {}\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "lib.rs"],
        vec![
            "-c",
            "user.name=JevGate test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "baseline",
        ],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&project.0)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
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
fn absent_credentials_produce_operational_failure_with_atomic_report() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), "fn f() {}").unwrap();
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
    let output = project
        .command()
        .args([
            "auth",
            "status",
            "--offline",
            "--json",
            "--env-file",
            "missing.env",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["configured"], false);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("Selected --env-file")
    );
}

#[cfg(unix)]
#[test]
fn saved_credential_is_shared_across_repositories_and_logout_preserves_overrides() {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let project = Project::new();
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

#[test]
fn missing_saved_credential_has_machine_readable_actionable_status() {
    let project = Project::new();
    let output = project
        .command()
        .args(["auth", "status", "--offline", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["configured"], false);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("jevgate auth login")
    );
    assert!(body["authenticated"].is_null());
    assert!(!project.0.join(".jevgate").exists());
}

#[cfg(unix)]
#[test]
fn watcher_debounces_updates_and_releases_lock_after_credential_failure() {
    let project = Project::new();
    let source = project.0.join("lib.rs");
    std::fs::write(&source, "fn initial() {}").unwrap();
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
    std::fs::write(&source, "fn changed() {}").unwrap();
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
    let output = project.command().arg("rules").output().unwrap();
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
            "shared_logic"
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
