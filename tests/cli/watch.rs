//! Watching: debounced updates and the writer lock.
use super::*;
use std::time::{Duration, Instant};

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
