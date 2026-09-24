//! Credentials: `auth login`, `status` and `logout`, and a check without a key.
use super::*;

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
