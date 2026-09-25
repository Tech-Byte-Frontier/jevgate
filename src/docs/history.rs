//! What Git holds about a repository's paths: tracked files, release tags,
//! and paths since deleted or renamed. Evidence for staleness candidates.
use crate::revision::git;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// What Git holds about a repository's paths, for documentation checks.
#[derive(Debug, Default)]
pub struct History {
    pub tracked: BTreeSet<PathBuf>,
    pub tags: BTreeSet<String>,
    /// Paths no longer in the repository: deleted, or renamed to the path given.
    pub removed: BTreeMap<PathBuf, Option<PathBuf>>,
}

/// The paths of `paths` the repository's ignore files cover, such as
/// `.env.local` or a build output; none outside a Git repository. The paths
/// go through standard input, since `check-ignore` refuses the literal
/// pathspecs the other Git calls use.
pub fn ignored(root: &Path, paths: &[PathBuf]) -> BTreeSet<PathBuf> {
    use std::io::Write;
    let mut input = Vec::new();
    for path in paths.iter().filter(|p| !p.as_os_str().is_empty()) {
        input.extend_from_slice(path.to_string_lossy().as_bytes());
        input.push(0);
    }
    if input.is_empty() {
        return BTreeSet::new();
    }
    let child = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "--no-index", "--stdin", "-z"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return BTreeSet::new();
    };
    // Written apart from reading, so a long answer cannot block the input.
    let writer = child.stdin.take().map(|mut stdin| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        })
    });
    let output = child.wait_with_output();
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    // Git exits 1 when none is ignored.
    output
        .map(|out| {
            out.stdout
                .split(|b| *b == 0)
                .filter(|l| !l.is_empty())
                .map(|l| PathBuf::from(String::from_utf8_lossy(l).into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// Tracked paths, tags and removed paths; empty outside a Git repository.
pub fn history(root: &Path) -> History {
    let lines = |args: &[&str]| -> Vec<String> {
        git(root, args)
            .map(|out| {
                out.split(|b| *b == 0 || *b == b'\n')
                    .filter(|l| !l.is_empty())
                    .map(|l| String::from_utf8_lossy(l).into_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut history = History {
        tracked: lines(&["ls-files", "-z"])
            .into_iter()
            .map(PathBuf::from)
            .collect(),
        tags: lines(&["tag"]).into_iter().collect(),
        removed: BTreeMap::new(),
    };
    // Newest first, so a path's most recent removal wins.
    let fields = lines(&[
        "log",
        "--format=",
        "--name-status",
        "-z",
        "-M",
        "--diff-filter=DR",
    ]);
    let mut fields = fields.iter();
    while let Some(status) = fields.next() {
        let Some(path) = fields.next() else { break };
        let to = status
            .starts_with('R')
            .then(|| fields.next().map(PathBuf::from))
            .flatten();
        let path = PathBuf::from(path);
        if !history.tracked.contains(&path) {
            history.removed.entry(path).or_insert(to);
        }
    }
    history
}
