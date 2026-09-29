//! `jevgate init --git-hook pre-push|pre-commit`: write the Git hook that
//! runs `jevgate check --pre-push` or `--staged`. It writes only where Git
//! reads hooks itself, and never over a hook it did not write: when another
//! tool manages the repository's hooks (husky through `core.hooksPath`,
//! pre-commit or lefthook through scripts of their own), it says what to
//! add there instead.
use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use std::path::{Path, PathBuf};

/// The Git hooks JevGate writes.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum GitHook {
    /// Before each push, judge what it sends: `jevgate check --pre-push`
    PrePush,
    /// Before each commit, judge what is staged: `jevgate check --staged`
    PreCommit,
}

impl GitHook {
    /// Git's name for the hook, and its file's.
    pub fn name(self) -> &'static str {
        match self {
            Self::PrePush => "pre-push",
            Self::PreCommit => "pre-commit",
        }
    }

    /// The command the hook runs.
    pub fn command(self) -> &'static str {
        match self {
            Self::PrePush => "jevgate check --pre-push",
            Self::PreCommit => "jevgate check --staged",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::PrePush => "push",
            Self::PreCommit => "commit",
        }
    }
}

/// The line that marks a hook as JevGate's, so it rewrites or removes only
/// its own.
const MARKER: &str = "# Written by `jevgate init --git-hook`";

/// Where the recipes for other hook managers are.
const RECIPES: &str = "https://tech-byte-frontier.github.io/jevgate/git-hooks.html";

/// The hook's script: POSIX shell, which Git for Windows runs too. A
/// missing `jevgate` lets the commit or push through with a line saying so,
/// as a run that cannot finish does: a Git client started from a desktop
/// may not have the PATH a terminal has.
fn script(hook: GitHook) -> String {
    let (name, noun) = (hook.name(), hook.noun());
    format!(
        "#!/bin/sh\n\
         {MARKER} {name}.\n\
         # It stops a {noun} while findings fail JevGate's gate, and lets the {noun}\n\
         # through, saying so, when JevGate cannot finish or is not installed.\n\
         # `jevgate init --git-hook {name} --remove` takes it out.\n\
         if ! command -v jevgate >/dev/null 2>&1; then\n\
         \x20 echo \"jevgate: not found on PATH; this {noun} was not checked.\" >&2\n\
         \x20 exit 0\n\
         fi\n\
         exec {}\n",
        hook.command()
    )
}

/// What `init --git-hook` does to the hook file.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Wrote(PathBuf),
    Unchanged(PathBuf),
    Removed(PathBuf),
    Absent(PathBuf),
}

/// Write `hook` in the repository around `cwd`, or with `remove` take out
/// the one JevGate wrote; with `dry_run`, only say what would change.
pub fn install(cwd: &Path, hook: GitHook, remove: bool, dry_run: bool) -> Result<Outcome> {
    let command = hook.command();
    if let Some(managed) = hooks_path(cwd)? {
        bail!(
            "Git runs this repository's hooks from {managed} (core.hooksPath), which a hook manager such as husky keeps; add `{command}` to its {} hook there instead ({RECIPES})",
            hook.name()
        );
    }
    let path = hooks_directory(cwd)?.join(hook.name());
    let written = script(hook);
    let outcome = match std::fs::read_to_string(&path) {
        Ok(text) if text.contains(MARKER) && remove => Outcome::Removed(path),
        Ok(text) if text == written => Outcome::Unchanged(path),
        Ok(text) if text.contains(MARKER) => Outcome::Wrote(path),
        Ok(text) => bail!("{}", foreign(&path, &text, hook)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && remove => {
            Outcome::Absent(path)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Outcome::Wrote(path),
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot read {}", path.display()));
        }
    };
    if dry_run {
        return Ok(outcome);
    }
    match &outcome {
        Outcome::Wrote(path) => write(path, &written)?,
        Outcome::Removed(path) => std::fs::remove_file(path)
            .with_context(|| format!("Cannot remove {}", path.display()))?,
        Outcome::Unchanged(_) | Outcome::Absent(_) => {}
    }
    Ok(outcome)
}

/// `jevgate init --git-hook`: install or remove the hook, and say what
/// changed and what it needs.
pub fn run(hook: GitHook, remove: bool, dry_run: bool) -> Result<u8> {
    let cwd = std::env::current_dir()?.canonicalize()?;
    let outcome = install(&cwd, hook, remove, dry_run)?;
    let (verb, path) = match &outcome {
        Outcome::Wrote(path) if dry_run => ("Would write", path),
        Outcome::Wrote(path) => ("Wrote", path),
        Outcome::Unchanged(path) => ("Unchanged:", path),
        Outcome::Removed(path) if dry_run => ("Would remove", path),
        Outcome::Removed(path) => ("Removed", path),
        Outcome::Absent(path) => ("No JevGate hook to remove at", path),
    };
    say!("{verb} {}", path.display());
    if matches!(outcome, Outcome::Wrote(_) | Outcome::Unchanged(_)) {
        say!(
            "Before each {}, it runs `{}`. It needs an API key (jevgate auth login); when JevGate cannot finish, the {} goes ahead and it says so.",
            hook.noun(),
            hook.command(),
            hook.noun()
        );
    }
    Ok(0)
}

/// Why a hook JevGate did not write stays, and what to add to it.
fn foreign(path: &Path, text: &str, hook: GitHook) -> String {
    let command = hook.command();
    let shown = path.display();
    if text.contains("pre-commit.com") || text.contains("prek") {
        return format!(
            "pre-commit or prek wrote {shown}; add JevGate to .pre-commit-config.yaml instead, as the hook `jevgate-push-system` (before a push) or `jevgate-system` (before a commit) ({RECIPES})"
        );
    }
    if text.contains("lefthook") {
        return format!(
            "lefthook wrote {shown}; add `{command}` to lefthook.yml instead ({RECIPES})"
        );
    }
    format!(
        "{shown} is a hook JevGate did not write; add `{command}` to it, or move it away and run this again"
    )
}

/// The directory `core.hooksPath` names, as Git reads it; none when unset.
fn hooks_path(cwd: &Path) -> Result<Option<String>> {
    let output = crate::revision::git_in(cwd)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .context("Cannot run Git")?;
    // Exit 1: the key is not set.
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((output.status.success() && !value.is_empty()).then_some(value))
}

/// The directory Git reads the repository's hooks from: `.git/hooks`, or
/// the main work tree's from a linked one.
fn hooks_directory(cwd: &Path) -> Result<PathBuf> {
    let bytes = crate::revision::git(cwd, &["rev-parse", "--git-path", "hooks"])
        .context("jevgate init --git-hook writes a Git hook; run it inside a Git work tree")?;
    let relative = String::from_utf8(bytes)?.trim().to_string();
    Ok(cwd.join(relative))
}

/// Write the hook, executable where the file system says so.
fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)
            .with_context(|| format!("Cannot create {}", directory.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("Cannot write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("Cannot make {} executable", path.display()))?;
    }
    Ok(())
}
