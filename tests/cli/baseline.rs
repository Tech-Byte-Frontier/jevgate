//! `baseline list` and `baseline mark --note` on a committed baseline.
use super::*;

/// A baseline written before notes and units: two files, one finding
/// without a reason.
const OLD_BASELINE: &str = r#"{
  "version": 1,
  "created_at": 1790976832,
  "findings": [
    {
      "fingerprint": "bbbbbbbb22222222",
      "rule": "maintainability/shared-logic",
      "path": "src/b.ts",
      "line": 40,
      "strength": "review",
      "message": "Copies.",
      "reason": "later"
    },
    {
      "fingerprint": "aaaaaaaa11111111",
      "rule": "maintainability/function-simplification",
      "path": "src/a.ts",
      "line": 7,
      "strength": "review",
      "message": "Long.",
      "reason": "wrong"
    },
    {
      "fingerprint": "cccccccc33333333",
      "rule": "maintainability/shared-logic",
      "path": "src/a.ts",
      "line": 3,
      "message": "Copies."
    }
  ]
}
"#;

fn run(project: &Project, args: &[&str]) -> std::process::Output {
    project.command().args(args).output().unwrap()
}

fn stdout(project: &Project, args: &[&str]) -> String {
    let output = run(project, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn baseline_list_prints_text_markdown_and_json_by_path_then_line() {
    let project = Project::new();
    std::fs::write(project.0.join("jevgate-baseline.json"), OLD_BASELINE).unwrap();
    assert_eq!(
        stdout(&project, &["baseline", "list"]),
        "\
src/a.ts:3   no reason  maintainability/shared-logic             cccccccc
src/a.ts:7   wrong      maintainability/function-simplification  aaaaaaaa
src/b.ts:40  later      maintainability/shared-logic             bbbbbbbb
"
    );
    assert_eq!(
        stdout(
            &project,
            &["baseline", "list", "--reason", "later", "--format", "md"]
        ),
        "- [ ] `src/b.ts:40` maintainability/shared-logic (later, `bbbbbbbb`)\n"
    );
    let json: serde_json::Value = serde_json::from_str(&stdout(
        &project,
        &[
            "baseline",
            "list",
            "--rule",
            "shared-logic",
            "--format",
            "json",
        ],
    ))
    .unwrap();
    let ids: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["fingerprint"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["cccccccc33333333", "bbbbbbbb22222222"]);
    assert_eq!(json[1]["reason"], "later");
    assert_eq!(json[0]["reason"], serde_json::Value::Null);

    // A filter that matches nothing, and an unknown rule.
    for format in ["text", "md"] {
        assert_eq!(
            stdout(
                &project,
                &[
                    "baseline", "list", "--reason", "intended", "--format", format
                ]
            ),
            "No accepted findings match.\n"
        );
    }
    assert_eq!(
        stdout(
            &project,
            &["baseline", "list", "--rule", "security", "--format", "json"]
        ),
        "[]\n"
    );
    assert!(
        !run(&project, &["baseline", "list", "--rule", "nonsense"])
            .status
            .success()
    );

    // An empty baseline lists nothing in each format.
    std::fs::write(
        project.0.join("jevgate-baseline.json"),
        r#"{"version": 1, "created_at": 0, "findings": []}"#,
    )
    .unwrap();
    assert_eq!(
        stdout(&project, &["baseline", "list"]),
        "No accepted findings match.\n"
    );
    assert_eq!(
        stdout(&project, &["baseline", "list", "--format", "md"]),
        "No accepted findings match.\n"
    );
    assert_eq!(
        stdout(&project, &["baseline", "list", "--format", "json"]),
        "[]\n"
    );
}

#[test]
fn a_mark_note_is_kept_in_the_baseline_and_listed() {
    let project = Project::new();
    let path = project.0.join("jevgate-baseline.json");
    std::fs::write(&path, OLD_BASELINE).unwrap();
    let marked = stdout(
        &project,
        &["baseline", "mark", "later", "--note", "#192", "src/a.ts:3"],
    );
    assert_eq!(marked, "Marked 1 finding as later\n");
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let old: serde_json::Value = serde_json::from_str(OLD_BASELINE).unwrap();
    let entry = |value: &serde_json::Value, id: &str| {
        value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["fingerprint"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(entry(&written, "cccccccc33333333")["note"], "#192");
    // The entries it did not mark are written as they were.
    for id in ["aaaaaaaa11111111", "bbbbbbbb22222222"] {
        assert_eq!(entry(&written, id), entry(&old, id));
    }
    assert_eq!(
        stdout(
            &project,
            &["baseline", "list", "--format", "md", "--reason", "later"]
        ),
        "\
- [ ] `src/a.ts:3` maintainability/shared-logic (later, `cccccccc`): #192
- [ ] `src/b.ts:40` maintainability/shared-logic (later, `bbbbbbbb`)
"
    );
    // A note of two lines is refused, and the baseline left as it was.
    let refused = run(
        &project,
        &["baseline", "mark", "wrong", "--note", "a\nb", "src/a.ts:3"],
    );
    assert!(!refused.status.success());
    assert_eq!(
        entry(
            &serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            "cccccccc33333333"
        )["reason"],
        "later"
    );

    // The unit a finding names follows the columns, before the note.
    let text = std::fs::read_to_string(&path).unwrap().replace(
        r#""line": 3,"#,
        r#""line": 3, "unit": "`total` (src/a.ts:3) and `sum` (src/b.ts:40)","#,
    );
    let text = text.replace(r#""line": 7,"#, r#""line": 7, "unit": "total","#);
    std::fs::write(&path, text).unwrap();
    assert_eq!(
        stdout(&project, &["baseline", "list", "--reason", "later"]),
        "\
src/a.ts:3   later  maintainability/shared-logic  cccccccc  `total` (src/a.ts:3) and `sum` (src/b.ts:40)  note: #192
src/b.ts:40  later  maintainability/shared-logic  bbbbbbbb
"
    );
    assert_eq!(
        stdout(&project, &["baseline", "list", "--format", "md"]),
        "\
- [ ] `src/a.ts:3` maintainability/shared-logic `total` (src/a.ts:3) and `sum` (src/b.ts:40) (later, `cccccccc`): #192
- [ ] `src/a.ts:7` maintainability/function-simplification `total` (wrong, `aaaaaaaa`)
- [ ] `src/b.ts:40` maintainability/shared-logic (later, `bbbbbbbb`)
"
    );
}
