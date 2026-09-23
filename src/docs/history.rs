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
