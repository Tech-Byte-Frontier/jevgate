//! A warning when Git ignores the question files.
use std::{path::Path, process::Command};

/// The warning to print when Git ignores `file`, a question file relative
/// to `root`. A root `.gitignore` entry such as `/.jevgate/` hides
/// `.jevgate/questions/` from commits, and the `.gitignore` JevGate writes
/// inside `.jevgate/` cannot re-include a directory its parent excludes: the
/// gate would then ask the questions locally and never in CI. None outside
/// a Git repository, when Git is missing, or when the file is not ignored.
pub fn ignored(root: &Path, file: &Path) -> Option<String> {
    // Exit 0 only when ignored: 1 is not ignored, 128 no repository.
    check_ignore(root, file, &[])?;
    // Verbose output also names a negation that keeps a file, so it only
    // says which rule ignores it.
    let matched = check_ignore(root, file, &["--verbose"]).unwrap_or_default();
    let rule = matched.split('\t').next().unwrap_or_default().trim();
    Some(format!(
        "jevgate: Git ignores {} ({rule}), so its questions never reach a commit or CI; ignore `/.jevgate/*` with `!/.jevgate/questions/` instead",
        super::DIRECTORY
    ))
}

/// `git check-ignore` of `file` with `flags`: its output when it exits 0.
fn check_ignore(root: &Path, file: &Path, flags: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("check-ignore")
        .args(flags)
        .arg("--")
        .arg(file)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}
