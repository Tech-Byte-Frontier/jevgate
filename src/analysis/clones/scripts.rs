//! Scripts that share code only through the files they read in: a Bash
//! script runs on its own, so a copy in another script that it does not
//! `source` has nowhere shared to live. setup-ipsec-vpn's scripts are each
//! fetched by URL and run alone, and 31 of the 33 copies 0.30 found between
//! them were labeled wrong or debatable.
use super::{SourceFile, generic};
use std::collections::BTreeSet;

pub(super) struct Scripts {
    /// For each file, the names of the scripts it reads in; none for a
    /// language that shares code otherwise (`generic::includes`).
    includes: Vec<Option<BTreeSet<String>>>,
    /// Each file's name.
    names: Vec<String>,
    /// The names of the scripts in scope, which a shared file must be.
    project: BTreeSet<String>,
}

impl Scripts {
    pub(super) fn of(files: &[SourceFile<'_>]) -> Self {
        let includes: Vec<Option<BTreeSet<String>>> = files
            .iter()
            .map(|file| generic::includes(file.path, file.source))
            .collect();
        let names: Vec<String> = files
            .iter()
            .map(|file| {
                file.path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        let project = names
            .iter()
            .zip(&includes)
            .filter(|(_, included)| included.is_some())
            .map(|(name, _)| name.clone())
            .collect();
        Self {
            includes,
            names,
            project,
        }
    }

    /// Whether copies in files `a` and `b` can share one implementation:
    /// always, unless both are such scripts; then when they are one script,
    /// one reads the other in, or both read in the same script of the
    /// project, which could hold it.
    pub(super) fn linked(&self, a: usize, b: usize) -> bool {
        let (Some(x), Some(y)) = (&self.includes[a], &self.includes[b]) else {
            return true;
        };
        a == b
            || x.contains(&self.names[b])
            || y.contains(&self.names[a])
            || x.intersection(y).any(|name| self.project.contains(name))
    }
}
