//! `jevgate init --agent` as a person runs it: an agent's files under a
//! scratch home, written once, taken out, a repository's files, and what is
//! refused.
use super::*;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Output,
};

/// The directory holding the `jevgate` under test, so the `PATH` check finds it.
fn this_jevgate() -> OsString {
    Path::new(env!("CARGO_BIN_EXE_jevgate"))
        .parent()
        .unwrap()
        .as_os_str()
        .to_owned()
}

/// `jevgate init ARGS` in `dir`, with `home` as the home directory and `path` as `PATH`.
fn init(project: &Project, dir: &Path, path: &OsString, args: &[&str]) -> Output {
    project
        .command()
        .current_dir(dir)
        .arg("init")
        .args(args)
        .env("HOME", project.0.join("home"))
        .env("USERPROFILE", project.0.join("home"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("PATH", path)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn home(project: &Project, path: &str) -> PathBuf {
    project.0.join("home").join(path)
}

#[test]
fn an_agent_is_set_up_once_and_taken_out() {
    let project = Project::new();
    let path = this_jevgate();
    let first = init(&project, &project.0, &path, &["--agent", "claude,codex"]);
    let stdout = text(&first.stdout);
    assert!(first.status.success(), "{}", text(&first.stderr));
    assert!(
        stdout.contains("Claude Code, for your user (every repository):")
            && stdout
                .contains("settings.json: hooks SessionStart, UserPromptSubmit, PostToolUse, Stop")
            && stdout.contains("trust JevGate's"),
        "{stdout}"
    );
    assert_eq!(
        text(&first.stderr),
        "",
        "the jevgate on PATH answers the hooks"
    );
    let settings = std::fs::read_to_string(home(&project, ".claude/settings.json")).unwrap();
    assert!(
        settings.contains("\"command\": \"jevgate hook\""),
        "{settings}"
    );
    assert!(home(&project, ".codex/AGENTS.md").is_file());
    let again = init(&project, &project.0, &path, &["--agent", "claude,codex"]);
    let stdout = text(&again.stdout);
    assert!(
        !stdout.contains("created") && !stdout.contains("updated"),
        "{stdout}"
    );
    let removed = init(
        &project,
        &project.0,
        &path,
        &["--agent", "claude,codex", "--remove"],
    );
    assert!(removed.status.success());
    assert!(text(&removed.stdout).contains("removed"));
    for file in [
        ".claude/settings.json",
        ".claude/rules/jevgate.md",
        ".codex/hooks.json",
        ".codex/AGENTS.md",
    ] {
        assert!(!home(&project, file).exists(), "{file}");
    }
}

#[test]
fn a_dry_run_writes_nothing() {
    let project = Project::new();
    let output = init(
        &project,
        &project.0,
        &this_jevgate(),
        &["--agent", "gemini", "--dry-run"],
    );
    assert!(output.status.success());
    assert!(
        text(&output.stdout).contains("would create"),
        "{}",
        text(&output.stdout)
    );
    assert!(!home(&project, ".gemini").exists());
}

#[test]
fn a_repositorys_files_go_at_the_top_of_its_work_tree() {
    let project = Project::new();
    git(&project, &["init", "-q"]);
    let nested = project.0.join("src/deep");
    std::fs::create_dir_all(&nested).unwrap();
    let output = init(
        &project,
        &nested,
        &this_jevgate(),
        &["--agent", "cursor", "--project"],
    );
    let stdout = text(&output.stdout);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(project.0.join(".cursor/hooks.json").is_file(), "{stdout}");
    let rule = std::fs::read_to_string(project.0.join(".cursor/rules/jevgate.mdc")).unwrap();
    assert!(rule.starts_with("---\ndescription:") && rule.contains("alwaysApply: true"));
    assert!(stdout.contains("No jevgate.toml in"), "{stdout}");
    assert!(!nested.join(".cursor").exists());
}

#[test]
fn a_file_it_cannot_read_whole_leaves_every_file_as_it_was() {
    let project = Project::new();
    let gemini = home(&project, ".gemini/settings.json");
    std::fs::create_dir_all(gemini.parent().unwrap()).unwrap();
    let mine = "{\n  // my theme\n  \"theme\": \"dark\"\n}\n";
    std::fs::write(&gemini, mine).unwrap();
    let output = init(
        &project,
        &project.0,
        &this_jevgate(),
        &["--agent", "claude,gemini"],
    );
    assert_eq!(output.status.code(), Some(2));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("nothing was written") && stderr.contains("not plain JSON"),
        "{stderr}"
    );
    assert_eq!(std::fs::read_to_string(&gemini).unwrap(), mine);
    assert!(!home(&project, ".claude").exists());
}

#[test]
fn a_jevgate_missing_from_path_is_warned_about() {
    let project = Project::new();
    let empty = project.0.join("empty-bin");
    std::fs::create_dir_all(&empty).unwrap();
    let output = init(
        &project,
        &project.0,
        &empty.into_os_string(),
        &["--agent", "claude"],
    );
    assert!(output.status.success());
    let stderr = text(&output.stderr);
    assert!(stderr.contains("no jevgate is on your PATH"), "{stderr}");
}

#[test]
fn agent_options_are_refused_where_they_mean_nothing() {
    let project = Project::new();
    for args in [
        &["--agent", "claude", "--force"][..],
        &["--project"][..],
        &["--remove"][..],
        &["--agent", "vscode"][..],
    ] {
        let output = init(&project, &project.0, &this_jevgate(), args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
    assert!(!project.0.join("jevgate.toml").exists());
}
