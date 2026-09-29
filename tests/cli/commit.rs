//! Checks for Git hooks: `--staged` judges what a commit records and
//! `--pre-push` what a push sends, a run that cannot finish lets the change
//! through and says so, budgets stop the asking, and `init --git-hook`
//! writes the hooks that run them.
use super::*;
use mock_provider::{MockProvider, Reply, answer};
use serde_json::{Value, json};
use std::process::Output;

/// A provider whose answers make the long function a review and clear the rest.
fn reviewing() -> MockProvider {
    MockProvider::start(|received| {
        let level = if received.body.contains("smallest") {
            2
        } else {
            0
        };
        Reply::json(200, &answer(&received.json(), level))
    })
}

/// A provider that answers every request with HTTP 402.
fn exhausted() -> MockProvider {
    MockProvider::start(|_| Reply::json(402, &json!({"error": "credits"})))
}

/// Run `command` with `input` on stdin.
fn fed(command: &mut Command, input: &str) -> Output {
    use std::io::Write;
    let mut child = command
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
    child.wait_with_output().unwrap()
}

fn json_report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// The line of each finding in `report`, by path.
fn findings(report: &Value) -> Vec<(String, u64)> {
    report["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|file| {
            file["findings"].as_array().unwrap().iter().map(|finding| {
                (
                    file["path"].as_str().unwrap().to_string(),
                    finding["line"].as_u64().unwrap(),
                )
            })
        })
        .collect()
}

/// A committed project whose index holds the long function in `lib.rs`,
/// while the working tree holds a short one two lines lower, and an
/// untracked file holds the long one too.
fn staged_long() -> Project {
    let project = Project::committed();
    std::fs::write(project.0.join("lib.rs"), LONG_RS).unwrap();
    git(&project, &["add", "lib.rs"]);
    std::fs::write(project.0.join("lib.rs"), format!("// a\n// b\n{JUDGED_RS}")).unwrap();
    std::fs::write(project.0.join("loose.rs"), LONG_RS).unwrap();
    project
}

#[test]
fn staged_judges_what_the_index_holds_at_the_lines_it_holds_them() {
    let project = staged_long();
    let provider = reviewing();
    let output = project
        .asking(&provider, "key")
        .args(["check", "--staged", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let report = json_report(&output);
    assert_eq!(report["staged"], true);
    assert_eq!(
        findings(&report),
        [("lib.rs".to_string(), 1)],
        "the index's function at its line; the untracked file is not judged"
    );
    let text = project
        .asking(&provider, "key")
        .args(["check", "--staged"])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&text.stdout);
    assert!(said.contains(" · staged lines since "), "{said}");
    assert!(said.contains("JevGate stopped this commit"), "{said}");
    assert!(
        said.contains("never bypass this check with --no-verify"),
        "{said}"
    );
    // Checked by `--base`, the working tree's short function passes, and
    // the untracked file fails.
    let base = project
        .asking(&provider, "key")
        .args(["check", "--base", "HEAD", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(findings(&json_report(&base)), [("loose.rs".to_string(), 1)]);
}

#[test]
fn staged_reads_the_index_git_commits_from() {
    // `git commit PATHS` and `commit -a` commit through a temporary index,
    // which Git names to the pre-commit hook in GIT_INDEX_FILE.
    let project = Project::committed();
    std::fs::write(project.0.join("lib.rs"), LONG_RS).unwrap();
    let index = project.0.join(".git/next-index");
    std::fs::copy(project.0.join(".git/index"), &index).unwrap();
    let staged = git::command(&project.0)
        .env("GIT_INDEX_FILE", &index)
        .args(["add", "lib.rs"])
        .status()
        .unwrap();
    assert!(staged.success());
    let provider = reviewing();
    let check = |index: Option<&std::path::Path>| {
        let mut command = project.asking(&provider, "key");
        if let Some(index) = index {
            command.env("GIT_INDEX_FILE", index);
        }
        command
            .args(["check", "--staged", "--format", "json"])
            .output()
            .unwrap()
    };
    let committing = check(Some(&index));
    assert_eq!(committing.status.code(), Some(1), "{}", stderr(&committing));
    let unstaged = check(None);
    assert_eq!(unstaged.status.code(), Some(0), "{}", stderr(&unstaged));
    assert_eq!(json_report(&unstaged)["status"], "no-changed-source");
}

/// A clone of `project` with a bare remote holding its `main`, and the
/// long function committed on `feature`, then replaced on disk by a short one.
fn pushed_long(project: &Project) -> temp_dir::TempDir {
    let remote = temp_dir::TempDir::new("jevgate-remote");
    git::run(&remote, &["init", "-q", "--bare"]);
    git(project, &["branch", "-M", "main"]);
    git(
        project,
        &[
            "remote",
            "add",
            "origin",
            super::changes::plain(&remote).to_str().unwrap(),
        ],
    );
    git(project, &["push", "-q", "origin", "main"]);
    git(project, &["checkout", "-qb", "feature"]);
    std::fs::write(project.0.join("lib.rs"), LONG_RS).unwrap();
    git(project, &["commit", "-qam", "long"]);
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    remote
}

/// The pre-push line of `local` pushed over a remote ref at `remote`.
fn push_line(project: &Project, local: &str, remote: &str) -> String {
    let id = |revision: &str| {
        git::run(&project.0, &["rev-parse", revision])
            .trim()
            .to_string()
    };
    format!(
        "refs/heads/feature {} refs/heads/feature {}\n",
        id(local),
        id(remote)
    )
}

#[test]
fn pre_push_judges_the_pushed_commit_not_the_working_tree() {
    let project = Project::committed();
    let _remote = pushed_long(&project);
    let provider = reviewing();
    let line = push_line(&project, "feature", "main");
    let output = fed(
        project
            .asking(&provider, "key")
            .args(["check", "--pre-push", "--format", "json"]),
        &line,
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let report = json_report(&output);
    let head = git::run(&project.0, &["rev-parse", "HEAD"]);
    assert_eq!(report["pushed_revision"], head.trim());
    assert_eq!(findings(&report), [("lib.rs".to_string(), 1)]);
    // Under pre-commit, the ref comes in its variables.
    let named = project
        .asking(&provider, "key")
        .env("PRE_COMMIT_TO_REF", head.trim())
        .env("PRE_COMMIT_FROM_REF", "main")
        .args(["check", "--pre-push", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(named.status.code(), Some(1), "{}", stderr(&named));
    let nothing = fed(
        project
            .asking(&provider, "key")
            .args(["check", "--pre-push"]),
        &push_line(&project, "main", "main"),
    );
    assert_eq!(nothing.status.code(), Some(0));
    assert!(
        stderr(&nothing).contains("nothing to check: this push sends no commit a remote lacks"),
        "{}",
        stderr(&nothing)
    );
}

#[test]
fn a_commit_or_a_push_goes_ahead_when_the_check_cannot_finish_and_says_so() {
    let project = staged_long();
    let provider = exhausted();
    let check = |args: &[&str]| {
        project
            .asking(&provider, "key")
            .arg("check")
            .args(args)
            .output()
            .unwrap()
    };
    let passed = check(&["--staged"]);
    assert_eq!(passed.status.code(), Some(0));
    let said = stderr(&passed);
    assert!(
        said.contains("jevgate: this commit was not checked: TypeSafe HTTP 402"),
        "{said}"
    );
    assert!(
        said.contains("it goes ahead unchecked, as on_incomplete is \"pass\""),
        "{said}"
    );
    assert_eq!(
        check(&["--staged", "--on-incomplete", "fail"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        check(&["--base", "HEAD"]).status.code(),
        Some(2),
        "CI keeps exit 2"
    );
    let opted = check(&["--base", "HEAD", "--on-incomplete", "pass"]);
    assert_eq!(opted.status.code(), Some(0));
    assert!(stderr(&opted).contains("jevgate: the check did not finish: TypeSafe HTTP 402"));
    std::fs::write(project.0.join("jevgate.toml"), "on_incomplete = \"fail\"\n").unwrap();
    assert_eq!(check(&["--staged"]).status.code(), Some(2));
    // Without a key, or with a configuration that does not load, nothing
    // is judged, and the commit goes ahead all the same.
    std::fs::write(project.0.join("jevgate.toml"), "on_incomplete = \"pass\"\n").unwrap();
    let keyless = project
        .command()
        .args(["check", "--staged"])
        .output()
        .unwrap();
    assert_eq!(keyless.status.code(), Some(0));
    assert!(
        stderr(&keyless).contains("this commit was not checked: No API key configured"),
        "{}",
        stderr(&keyless)
    );
    std::fs::write(project.0.join("jevgate.toml"), "rules = 3\n").unwrap();
    let invalid = check(&["--staged"]);
    assert_eq!(invalid.status.code(), Some(0));
    assert!(stderr(&invalid).contains("this commit was not checked: Invalid"));
    assert_eq!(check(&[]).status.code(), Some(2));
}

#[test]
fn budgets_stop_the_asking_and_leave_the_run_incomplete() {
    let project = staged_long();
    let slow = MockProvider::start(|received| Reply {
        delay: std::time::Duration::from_secs(5),
        ..Reply::json(200, &answer(&received.json(), 0))
    });
    let started = std::time::Instant::now();
    let timed = project
        .asking(&slow, "key")
        .args(["check", "--staged", "--max-seconds", "1"])
        .output()
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "an attempt under way gets only the time left"
    );
    assert_eq!(timed.status.code(), Some(0));
    assert!(
        stderr(&timed).contains("it used its 1 second (max_seconds) before every answer came back"),
        "{}",
        stderr(&timed)
    );
    let provider = reviewing();
    let priced = project
        .asking(&provider, "key")
        .args([
            "check",
            "--staged",
            "--max-cost",
            "0.0000001",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(priced.status.code(), Some(0));
    assert!(
        provider.received().is_empty(),
        "nothing is sent past the budget"
    );
    assert!(
        stderr(&priced).contains("would pass the $0.00 budget (max_cost)"),
        "{}",
        stderr(&priced)
    );
    let usage = project
        .command()
        .args(["check", "--max-cost", "0"])
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(2));
}

#[test]
fn the_hook_moments_do_not_combine_with_watching_or_another_base() {
    let project = Project::committed();
    for args in [
        &["--staged", "--watch"][..],
        &["--staged", "--base", "HEAD"],
        &["--pre-push", "--staged"],
    ] {
        let output = project.command().arg("check").args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
    let whole = project
        .command()
        .args(["check", "--staged", "--whole-files", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(whole.status.code(), Some(0), "{}", stderr(&whole));
}

#[test]
fn init_writes_its_own_git_hook_and_no_other() {
    let project = Project::committed();
    let init = |args: &[&str]| {
        project
            .command()
            .args(["init", "--git-hook"])
            .args(args)
            .output()
            .unwrap()
    };
    let hook = project.0.join(".git/hooks/pre-push");
    let written = init(&["pre-push"]);
    assert_eq!(written.status.code(), Some(0), "{}", stderr(&written));
    let text = std::fs::read_to_string(&hook).unwrap();
    assert!(text.starts_with("#!/bin/sh\n"));
    assert!(text.contains("exec jevgate check --pre-push\n"), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&hook).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "executable");
    }
    let again = init(&["pre-push"]);
    assert!(String::from_utf8_lossy(&again.stdout).starts_with("Unchanged:"));
    let dry = init(&["pre-push", "--remove", "--dry-run"]);
    assert!(String::from_utf8_lossy(&dry.stdout).starts_with("Would remove"));
    assert!(hook.exists());
    assert_eq!(init(&["pre-push", "--remove"]).status.code(), Some(0));
    assert!(!hook.exists());
    let commit_hook = project.0.join(".git/hooks/pre-commit");
    for (text, said) in [
        ("#!/bin/sh\nmake lint\n", "is a hook JevGate did not write"),
        (
            "#!/usr/bin/env bash\n# File generated by pre-commit: https://pre-commit.com\n",
            ".pre-commit-config.yaml",
        ),
    ] {
        std::fs::write(&commit_hook, text).unwrap();
        let refused = init(&["pre-commit"]);
        assert_eq!(refused.status.code(), Some(2));
        assert!(stderr(&refused).contains(said), "{}", stderr(&refused));
        assert_eq!(std::fs::read_to_string(&commit_hook).unwrap(), text);
    }
    git(&project, &["config", "core.hooksPath", ".husky/_"]);
    let managed = init(&["pre-push"]);
    assert_eq!(managed.status.code(), Some(2));
    assert!(
        stderr(&managed).contains("core.hooksPath"),
        "{}",
        stderr(&managed)
    );
    let usage = project
        .command()
        .args(["init", "--remove"])
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(2));
}

/// Git in `project` as a person runs it, with the `jevgate` under test
/// first on PATH, its requests going to `provider`.
#[cfg(unix)]
fn git_with_hooks(project: &Project, provider: &MockProvider, args: &[&str]) -> Output {
    let binary = std::path::Path::new(env!("CARGO_BIN_EXE_jevgate"));
    let path = std::env::join_paths(
        std::iter::once(binary.parent().unwrap().to_path_buf()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .unwrap();
    git::command(&project.0)
        .args(args)
        .env("PATH", path)
        .env("TYPESAFE_API_KEY", "key")
        .env("JEVGATE_BASE_URL", &provider.url)
        .env("JEVGATE_CREDENTIAL_STORE", "file")
        .env("JEVGATE_CONFIG_DIR", project.0.join("isolated-auth"))
        .env_remove("CI")
        .output()
        .unwrap()
}

#[cfg(unix)]
#[test]
fn the_hooks_stop_a_commit_and_a_push_and_let_through_what_they_cannot_check() {
    let project = Project::committed();
    let init = |hook: &str| {
        let output = project
            .command()
            .args(["init", "--git-hook", hook])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    };
    init("pre-commit");
    std::fs::write(project.0.join("lib.rs"), LONG_RS).unwrap();
    git(&project, &["add", "lib.rs"]);
    let commits = || git::run(&project.0, &["rev-list", "--count", "HEAD"]);
    let before = commits();
    let stopped = git_with_hooks(&project, &reviewing(), &["commit", "-qm", "long"]);
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&stopped.stdout),
        stderr(&stopped)
    );
    assert!(!stopped.status.success(), "{said}");
    assert!(said.contains("JevGate stopped this commit"), "{said}");
    assert_eq!(commits(), before);
    // Answers already given come from the cache; a new function is asked.
    std::fs::write(project.0.join("lib.rs"), LONG_RS.replace("+ 1", "+ 2")).unwrap();
    git(&project, &["add", "lib.rs"]);
    let through = git_with_hooks(&project, &exhausted(), &["commit", "-qm", "long"]);
    assert!(through.status.success(), "{}", stderr(&through));
    assert!(stderr(&through).contains("this commit was not checked"));
    assert_ne!(commits(), before);
    // Before a push: the commit just made is what it sends.
    let remote = temp_dir::TempDir::new("jevgate-remote");
    git::run(&remote, &["init", "-q", "--bare"]);
    git(
        &project,
        &[
            "remote",
            "add",
            "origin",
            super::changes::plain(&remote).to_str().unwrap(),
        ],
    );
    git(
        &project,
        &["push", "-q", "origin", "HEAD~1:refs/heads/main"],
    );
    git(&project, &["fetch", "-q", "origin"]);
    init("pre-push");
    let held = git_with_hooks(
        &project,
        &reviewing(),
        &["push", "-q", "origin", "HEAD:feature"],
    );
    assert!(!held.status.success());
    assert!(
        String::from_utf8_lossy(&held.stdout).contains("JevGate stopped this push")
            || stderr(&held).contains("JevGate stopped this push"),
        "{}",
        stderr(&held)
    );
    // Fixed and committed, the push goes: it no longer sends the long function.
    let clear = MockProvider::start(|received| Reply::json(200, &answer(&received.json(), 0)));
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS.replace("+ 1", "+ 3")).unwrap();
    git(&project, &["add", "lib.rs"]);
    let fixed = git_with_hooks(&project, &clear, &["commit", "-qm", "short"]);
    assert!(fixed.status.success(), "{}", stderr(&fixed));
    let pushed = git_with_hooks(&project, &clear, &["push", "-q", "origin", "HEAD:feature"]);
    assert!(pushed.status.success(), "{}", stderr(&pushed));
}

#[test]
fn a_provider_failure_is_waited_out_by_the_next_commits() {
    let project = staged_long();
    let down = MockProvider::start(|_| Reply::json(503, &json!({"error": "overloaded"})));
    let first = project
        .asking(&down, "key")
        .args(["check", "--staged", "--max-seconds", "2"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert!(project.0.join(".jevgate/turns/outage.json").exists());
    // The next commit asks nothing for a few minutes, and says why.
    std::fs::write(project.0.join("lib.rs"), LONG_RS.replace("+ 1", "+ 5")).unwrap();
    git(&project, &["add", "lib.rs"]);
    let asked = down.received().len();
    let started = std::time::Instant::now();
    let next = project
        .asking(&down, "key")
        .args(["check", "--staged"])
        .output()
        .unwrap();
    assert_eq!(next.status.code(), Some(0));
    assert_eq!(down.received().len(), asked, "nothing sent while waiting");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(
        stderr(&next)
            .contains("this commit was not checked: the provider failed a few minutes ago"),
        "{}",
        stderr(&next)
    );
}

#[test]
fn a_push_of_several_refs_checks_each_distinct_change_once() {
    let project = Project::committed();
    let _remote = pushed_long(&project);
    git(&project, &["checkout", "-q", "--", "lib.rs"]);
    git(&project, &["checkout", "-qb", "other", "main"]);
    std::fs::write(
        project.0.join("other.rs"),
        JUDGED_RS.replace("fn f(", "fn o("),
    )
    .unwrap();
    git(&project, &["add", "other.rs"]);
    git(&project, &["commit", "-qm", "other"]);
    let id = |revision: &str| {
        git::run(&project.0, &["rev-parse", revision])
            .trim()
            .to_string()
    };
    let lines = format!(
        "refs/heads/feature {feature} refs/heads/feature {zero}\nrefs/heads/copy {feature} refs/heads/copy {zero}\nrefs/heads/other {other} refs/heads/other {zero}\nrefs/heads/main {main} refs/heads/main {main}\n",
        feature = id("feature"),
        other = id("other"),
        main = id("main"),
        zero = "0".repeat(40),
    );
    let provider = reviewing();
    let output = fed(
        project
            .asking(&provider, "key")
            .args(["check", "--pre-push"]),
        &lines,
    );
    let said = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{said}{}", stderr(&output));
    assert_eq!(
        said.matches("JevGate: ").count(),
        2,
        "feature once for its two names, other once, main not at all: {said}"
    );
}
