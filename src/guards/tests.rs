//! The scan over a real change: what each finder reports, and what a move,
//! a rename, a generated file or a comment-only edit leaves out.
use super::*;
use crate::tests::{Project, args};

const APP: &str = "def total(values):\n    return sum(values)  # type: ignore\n";
const TESTS: &str = "from app import total\n\ndef test_total():\n    assert total([1, 2]) == 3\n\ndef test_empty():\n    assert total([]) == 0\n";

/// A committed Python project with tests and a configuration.
fn repository() -> Project {
    let project = Project::new();
    project.write("app.py", APP);
    project.write("tests/test_app.py", TESTS);
    project.write(
        "tests/test_old.py",
        "def test_one():\n    assert True\n\ndef test_two():\n    assert True\n",
    );
    project.write(
        "jevgate.toml",
        "rules = [\"default\"]\nfail_on = [\"review\"]\n",
    );
    project.write("moved.py", "x = compute()  # noqa: E501\n");
    project.git(&["init", "-q"]);
    project.git(&["add", "."]);
    project.git(&["commit", "-qm", "start"]);
    project
}

/// What the change since HEAD does, for the whole repository.
fn scanned(project: &Project) -> Scan {
    let mut args = args();
    args.base = Some("HEAD".into());
    scan(&project.0, &args, &Config::default(), &[])
}

fn described(scan: &Scan) -> Vec<String> {
    scan.guards.iter().map(Guard::describe).collect()
}

#[test]
fn a_change_reports_what_it_turns_off_and_the_tests_it_changes() {
    let project = repository();
    // The existing `type: ignore` line moves below a new one.
    project.write(
        "app.py",
        "import os  # noqa: F401\n\ndef total(values):\n    values = list(values)\n    return sum(values)  # type: ignore\n",
    );
    project.write(
        "tests/test_app.py",
        "import pytest\nfrom app import total\n\n@pytest.mark.skip(reason=\"flaky\")\ndef test_total():\n    assert total([1, 2])\n",
    );
    std::fs::remove_file(project.0.join("tests/test_old.py")).unwrap();
    project.write(
        "jevgate.toml",
        "# gate\nrules = [\"default\"]\nfail_on = []\n",
    );
    project.write(
        "jevgate-baseline.json",
        "{\"version\": 1, \"created_at\": 1, \"findings\": []}\n",
    );
    project.git(&["mv", "moved.py", "renamed.py"]);
    project.write(
        "web/api.generated.ts",
        "/* eslint-disable */\nexport const a = 1;\n",
    );
    let scan = scanned(&project);
    assert_eq!(
        described(&scan),
        [
            "app.py:1 turns off flake8 or Ruff here: import os # noqa: F401",
            "jevgate-baseline.json is added",
            "jevgate.toml is edited: fail_on",
            "tests/test_app.py removes or renames test `test_empty`",
            "tests/test_app.py:4 skips a test: @pytest.mark.skip(reason=\"flaky\")",
            "tests/test_old.py is deleted, removing 2 tests",
        ]
    );
    assert_eq!(scan.changed_tests.len(), 1);
    let changed = &scan.changed_tests[0];
    assert_eq!((changed.name.as_str(), changed.line), ("test_total", 4));
    assert!(changed.before.contains("== 3"));
    assert_eq!(
        summary(&scan.guards),
        "adds 1 suppression, skips 1 test, removes tests, edits jevgate.toml and edits jevgate-baseline.json"
    );
}

#[test]
fn nothing_is_reported_without_a_base_or_outside_the_scope() {
    let project = repository();
    project.write("app.py", &format!("{APP}import os  # noqa: F401\n"));
    assert!(
        scan(&project.0, &args(), &Config::default(), &[])
            .guards
            .is_empty()
    );
    let mut args = args();
    args.base = Some("HEAD".into());
    let tests_only = [project.0.join("tests")];
    assert!(
        scan(&project.0, &args, &Config::default(), &tests_only)
            .guards
            .is_empty()
    );
    let whole = scan(
        &project.0,
        &args,
        &Config::default(),
        &[project.0.join("app.py")],
    );
    assert_eq!(whole.guards.len(), 1);
}

#[test]
fn a_test_moved_to_another_file_of_the_change_is_not_removed() {
    let project = repository();
    // `test_one` and `test_empty` move to a new file; `test_two` is gone.
    std::fs::remove_file(project.0.join("tests/test_old.py")).unwrap();
    project.write(
        "tests/test_new.py",
        "def test_one():\n    assert True\n\ndef test_empty():\n    assert True\n",
    );
    project.write(
        "tests/test_app.py",
        "from app import total\n\ndef test_total():\n    assert total([1, 2]) == 3\n",
    );
    let scan = scanned(&project);
    assert_eq!(
        described(&scan),
        ["tests/test_old.py is deleted, removing 1 test"]
    );
    assert_eq!(scan.guards[0].text, "test_two");
}

#[test]
fn markers_quoted_in_strings_or_named_in_comments_turn_nothing_off() {
    let project = repository();
    project.write(
        "app.py",
        &format!("{APP}RULES = [\"# noqa\", \"@pytest.mark.skip\"]  # noqa: E501\n"),
    );
    project.write(
        "lib.rs",
        "/// Tests marked `#[ignore]` run with `--ignored`.\npub fn a() {}\n\n#[cfg(test)]\nmod tests {\n    const DATA: &str = \"#[ignore]\";\n    #[test]\n    #[ignore]\n    fn slow() {}\n}\n",
    );
    assert_eq!(
        described(&scanned(&project)),
        [
            "app.py:3 turns off flake8 or Ruff here: RULES = [\"# noqa\", \"@pytest.mark.skip\"] # noqa: E501",
            "lib.rs:8 skips a test: #[ignore]",
        ],
        "only the comment at the end of line 3 and the attribute on line 8"
    );
}

#[test]
fn edits_that_change_no_setting_are_not_reported() {
    let project = repository();
    project.write(
        "jevgate.toml",
        "# the gate\nrules = [\"default\"]   # all of it\nfail_on = [\"review\"]\n",
    );
    project.write("tests/test_app.py", &TESTS.replace("== 3", "== 3  # sum"));
    let scan = scanned(&project);
    assert!(scan.guards.is_empty(), "{:?}", described(&scan));
    assert_eq!(
        scan.changed_tests.len(),
        1,
        "a changed assertion is asked about"
    );
}

#[test]
fn allow_comments_and_baseline_entries_are_their_own_guards() {
    let project = Project::new();
    let entry = |fingerprint: &str, reason: &str| {
        format!(
            "{{\"fingerprint\": \"{fingerprint}\", \"rule\": \"r\", \"path\": \"lib.rs\", \"message\": \"m\"{reason}}}"
        )
    };
    let baseline = |entries: &[String]| {
        format!(
            "{{\"version\": 1, \"created_at\": 1, \"findings\": [{}]}}\n",
            entries.join(", ")
        )
    };
    project.write("lib.rs", "fn f() {}\n");
    project.write(
        "jevgate-baseline.json",
        &baseline(&[entry("a", ""), entry("b", "")]),
    );
    project.git(&["init", "-q"]);
    project.git(&["add", "."]);
    project.git(&["commit", "-qm", "start"]);
    project.write(
        "lib.rs",
        "// jevgate: allow(shared_logic) mirrors g\nfn f() {}\n",
    );
    project.write(
        "jevgate-baseline.json",
        &baseline(&[
            entry("b", ", \"reason\": \"wrong\""),
            entry("c", ""),
            entry("d", ""),
        ]),
    );
    assert_eq!(
        described(&scanned(&project)),
        [
            "jevgate-baseline.json is edited: accepts 2 more findings, drops 1 finding and changes the reason of 1 finding",
            "lib.rs:1 accepts a finding: // jevgate: allow(shared_logic) mirrors g",
        ]
    );
}

#[test]
fn added_lines_leave_out_what_a_move_kept() {
    let before = "a\nb\nb\nc\n";
    let after = "c\nb\nnew\nb\nb\n";
    assert_eq!(added_lines(before, after), BTreeSet::from([2, 4]));
    assert_eq!(added_lines("", "x\n"), BTreeSet::from([0]));
}

#[test]
fn a_guard_keeps_its_id_when_its_line_moves() {
    let at = |line| {
        Guard::new(
            Kind::Suppression,
            Path::new("a.py"),
            Some(line),
            "x  # noqa",
            "m".into(),
        )
    };
    assert_eq!(at(3).id, at(30).id);
    assert_ne!(
        at(3).id,
        Guard::new(
            Kind::SkippedTest,
            Path::new("a.py"),
            Some(3),
            "x  # noqa",
            "m".into()
        )
        .id
    );
}
