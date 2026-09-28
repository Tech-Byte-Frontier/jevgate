//! `jevgate rules add`: gallery questions copied into a repository.
use super::*;

fn add(project: &Project, args: &[&str]) -> std::process::Output {
    project
        .command()
        .args(["rules", "add"])
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn an_added_question_is_listed_asked_and_tracked_by_git() {
    let project = Project::new();
    git(&project, &["init", "--quiet"]);
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    let output = add(&project, &["swallowed-errors", "n-plus-one"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("Added custom/swallowed-errors (function, review at 0.80) in .jevgate/questions/swallowed-errors.toml")
            && text.contains("Added custom/n-plus-one (function, consider at 0.80)"),
        "{text}"
    );
    let rules = project.command().arg("rules").output().unwrap();
    let table = String::from_utf8(rules.stdout).unwrap();
    assert!(
        table.contains("custom/swallowed-errors") && table.contains("custom/n-plus-one"),
        "{table}"
    );
    let report = dry_run(&project, &["--rule", "custom"]);
    assert_eq!(stages(&report), ["custom"]);
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&project.0)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    let status = String::from_utf8(status.stdout).unwrap();
    assert!(
        status.contains("?? .jevgate/questions/swallowed-errors.toml")
            && !status.contains(".jevgate/cache"),
        "the questions reach a commit and the state does not: {status}"
    );
    let again = add(&project, &["swallowed-errors"]);
    assert!(again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("Already added: custom/swallowed-errors")
    );
}

#[test]
fn an_unknown_name_lists_the_gallery_and_help_says_what_each_catches() {
    let project = Project::new();
    let output = add(&project, &["no-such-question"]);
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("possible values") && error.contains("swallowed-errors"),
        "{error}"
    );
    assert!(!project.0.join(".jevgate").exists());
    let help = add(&project, &["--help"]);
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(
        help.contains(
            "todo-without-owner: a TODO or FIXME that names neither an owner nor an issue."
        ),
        "{help}"
    );
}

#[test]
fn adding_warns_when_git_ignores_the_questions() {
    let project = Project::new();
    git(&project, &["init", "--quiet"]);
    std::fs::write(project.0.join(".gitignore"), "/.jevgate/\n").unwrap();
    let output = add(&project, &["resource-leak"]);
    assert!(output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("Git ignores .jevgate/questions (.gitignore:1:/.jevgate/)"),
        "{error}"
    );
}

#[test]
fn force_replaces_a_question_file_that_no_longer_loads() {
    let project = Project::new();
    std::fs::create_dir_all(project.0.join(".jevgate/questions")).unwrap();
    let broken = project.0.join(".jevgate/questions/thin-handlers.toml");
    std::fs::write(
        &broken,
        "question = \"Does it do business work?\"\nunit = \"function\"\nthreshold = 2\n",
    )
    .unwrap();
    let rules = project.command().arg("rules").output().unwrap();
    assert_eq!(
        rules.status.code(),
        Some(2),
        "every command that loads questions stops"
    );
    let refused = add(&project, &["thin-handlers"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--force replaces it"));
    let replaced = add(&project, &["--force", "thin-handlers"]);
    assert!(
        replaced.status.success(),
        "{}",
        String::from_utf8_lossy(&replaced.stderr)
    );
    let rules = project.command().arg("rules").output().unwrap();
    assert!(rules.status.success());
    assert!(String::from_utf8_lossy(&rules.stdout).contains("custom/thin-handlers"));
}
