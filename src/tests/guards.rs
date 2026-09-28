//! Guards in whole runs: a check with a base reports what its change does
//! to the checks around the code, and a dry run prices its questions.
use super::*;

#[test]
fn a_check_with_a_base_reports_its_guards_in_json_and_text() {
    let project = Project::new();
    project.write("app.py", "def total(values):\n    return sum(values)\n");
    project.git(&["init", "-q"]);
    project.git(&["add", "."]);
    project.git(&["commit", "-qm", "start"]);
    project.write(
        "app.py",
        "import os  # noqa: F401\n\ndef total(values):\n    return sum(values)\n",
    );
    let mut options = args();
    options.base = Some("HEAD".into());
    let report = run(&project, &options, &mut Mock::default());
    assert_eq!(
        serde_json::to_value(&report.guards).unwrap()[0],
        json!({"kind": "suppression", "path": "app.py", "line": 1,
            "text": "import os # noqa: F401", "message": "turns off flake8 or Ruff here",
            "id": report.guards[0].id})
    );
    let mut text = Vec::new();
    output::agent(&mut text, &report, false, output::Style::PLAIN).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert!(
        text.contains("\nGuards (1): changes to the checks around this code, for a person to look at; they never fail the gate\n  app.py:1 turns off flake8 or Ruff here: import os # noqa: F401\n"),
        "{text}"
    );
    assert!(
        report.gate.as_ref().unwrap().passed,
        "guards never fail the gate"
    );
}

#[test]
fn a_dry_run_lists_the_guards_and_prices_the_rewritten_tests() {
    let project = Project::new();
    project.write(
        "tests.rs",
        "#[test]\nfn adds() {\n    assert_eq!(1 + 1, 2);\n}\n",
    );
    project.git(&["init", "-q"]);
    project.git(&["add", "."]);
    project.git(&["commit", "-qm", "start"]);
    project.write(
        "tests.rs",
        "#[test]\n#[ignore]\nfn adds() {\n    assert!(1 + 1 > 0);\n}\n",
    );
    let mut options = args();
    options.base = Some("HEAD".into());
    options.dry_run = true;
    let (_, mut report) = snapshot(&project, &options);
    crate::evaluate::preview_guards(&mut report, &options, &project.context(), &[]);
    assert_eq!(
        report
            .guards
            .iter()
            .map(|g| g.describe())
            .collect::<Vec<_>>(),
        ["tests.rs:2 skips a test: #[ignore]"]
    );
    let stage = &report.stages["guards"];
    assert_eq!((stage.planned_requests, stage.planned_cached), (1, 0));
    assert!(stage.planned_tokens > 0, "priced");
}
