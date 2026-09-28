//! What earlier runs and people saved: proposals in `.jevgate/proposals/`,
//! questions accepted from them, and the ids in use; and writing new
//! proposals without replacing any file.
use super::{
    PROPOSALS,
    proposal::{Candidate, MARKER, Status},
};
use crate::custom::{self, Question};
use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
};

/// Bytes of `jevgate.toml` read for markers.
const CONFIG_BYTES: u64 = 1_048_576;

/// The markers of rules already proposed or accepted, and the ids in use.
#[derive(Default)]
pub struct Saved {
    /// Rules that are questions, with the question file's id when known.
    accepted: BTreeMap<String, Option<String>>,
    /// Rules proposed before, with their proposal's id.
    proposed: BTreeMap<String, String>,
    /// Ids no new proposal may take.
    taken: BTreeSet<String>,
}

impl Saved {
    /// Read the markers of the question files, `jevgate.toml` and the
    /// proposals of the repository at `root`, whose questions are `questions`.
    pub fn load(root: &Path, questions: &[Question]) -> Self {
        let mut saved = Self::default();
        saved
            .taken
            .extend(questions.iter().map(|q| q.id().to_string()));
        for (id, marker) in markers(&custom::directory(root)) {
            saved.accepted.insert(marker, Some(id));
        }
        let config = root.join(crate::init::CONFIG_FILE);
        if let Ok(text) = crate::inventory::read_source(&config, CONFIG_BYTES) {
            for marker in text.lines().filter_map(marker) {
                saved.accepted.entry(marker).or_insert(None);
            }
        }
        for (id, marker) in markers(&custom::within(root, PROPOSALS)) {
            saved.taken.insert(id.clone());
            saved.proposed.insert(marker, id);
        }
        saved
    }

    /// Where a rule with `marker` stands; a new one claims a free id from
    /// `id`, and a saved one takes its file's id.
    pub fn place(&mut self, id: &mut String, marker: &str) -> Status {
        if let Some(accepted) = self.accepted.get(marker) {
            if let Some(file) = accepted {
                id.clone_from(file);
            }
            return Status::Accepted;
        }
        if let Some(proposed) = self.proposed.get(marker) {
            id.clone_from(proposed);
            return Status::Kept;
        }
        let free = (1..)
            .map(|n| match n {
                1 => id.clone(),
                n => format!("{id}-{n}"),
            })
            .find(|candidate| !self.taken.contains(candidate))
            .expect("some suffix is free");
        self.taken.insert(free.clone());
        *id = free;
        Status::New
    }
}

/// The id and marker of each question file in `directory` that has one.
fn markers(directory: &Path) -> Vec<(String, String)> {
    let files = custom::question_files(directory).unwrap_or_default();
    files
        .iter()
        .filter_map(|path| {
            let id = path.file_stem()?.to_string_lossy().into_owned();
            let text = crate::inventory::read_source(path, custom::FILE_BYTES).ok()?;
            let marker = text.lines().find_map(marker)?;
            Some((id, marker))
        })
        .collect()
}

/// The hash a marker line holds.
fn marker(line: &str) -> Option<String> {
    let hash = line.trim().strip_prefix(MARKER)?.trim();
    (!hash.is_empty()).then(|| hash.to_string())
}

/// Write each new proposal to `.jevgate/proposals/<id>.toml`. A file that
/// appeared there since `Saved::load` is kept as it is, never replaced.
pub fn write(root: &Path, candidates: &mut [Candidate<'_>]) -> Result<()> {
    let directory = custom::within(root, PROPOSALS);
    let mut made = false;
    for candidate in candidates {
        let Some(proposal) = candidate.proposal.as_mut() else {
            continue;
        };
        if proposal.status != Status::New {
            continue;
        }
        if !made {
            crate::storage::real_directory(&directory, "Proposals must be a real directory")?;
            made = true;
        }
        let path: PathBuf = directory.join(format!("{}.toml", proposal.id));
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        match created {
            Ok(mut file) => file
                .write_all(proposal.file()?.as_bytes())
                .with_context(|| format!("Cannot write {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                proposal.status = Status::Kept;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Cannot write {}", path.display()));
            }
        }
    }
    Ok(())
}
