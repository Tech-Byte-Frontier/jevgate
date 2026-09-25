//! Request previews, the browser report and output.
use super::*;

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
fn openrouter_preview_maps_the_default_model_for_systemone_without_network() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    let report = project.preview(&[
        "check",
        "lib.rs",
        "--provider",
        "openrouter",
        "--dry-run",
        "--show-requests",
    ]);
    assert_eq!(report["initial_requests"][0]["model"], "jev-latest");
    assert_eq!(report["api_requests"], 0);
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
