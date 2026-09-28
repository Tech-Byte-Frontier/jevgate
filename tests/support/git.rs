//! Git for the tests' throwaway repositories, kept out of the repository the
//! tests run in. Shared by the unit tests (through `#[path]`) and the CLI
//! tests.
use std::{path::Path, process::Command};

/// Variables that point Git at one repository: its directory, work tree,
/// index, objects or ref namespace, or the subdirectory an alias started in.
/// Git exports some of them to what it runs: `GIT_DIR` to a hook,
/// `rebase --exec` or `bisect run` in a linked worktree, and
/// `GIT_INDEX_FILE` to a pre-commit hook (Git 2.51). A test's `git init`,
/// `add` or `commit` that inherits them acts on that repository instead of
/// its own: `git init` wrote `core.bare = true` into the configuration a
/// worktree shares with its main clone, and `add` and `commit` put a test's
/// files in the worktree's index and history.
const REPOSITORY_VARIABLES: [&str; 8] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
];

/// `command` without the variables that point Git at a repository, so the
/// Git it starts, itself or through `jevgate`, finds the repository from its
/// own directory.
pub fn isolate(command: &mut Command) -> &mut Command {
    for name in REPOSITORY_VARIABLES {
        command.env_remove(name);
    }
    command
}

/// Run Git in `dir`, isolated, with a fixed identity and no signing, and
/// return what it printed.
pub fn run(dir: &Path, args: &[&str]) -> String {
    let output = isolate(&mut Command::new("git"))
        .args([
            "-c",
            "user.name=JevGate test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
