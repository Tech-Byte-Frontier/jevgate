//! A session's turn under `.jevgate/turns/`: the working tree when the turn
//! began, how often its stops were blocked, what the agent was already told
//! and what it was not.
use crate::{revision, schema, storage};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

/// Turn files untouched this long belong to finished sessions and are removed.
const KEPT_SECS: u64 = 7 * 24 * 60 * 60;
/// A turn file is at most a few kilobytes; a larger one was not written here.
pub(super) const MAX_BYTES: u64 = 65_536;
/// Findings a turn remembers giving the agent: about 20 KB of fingerprints.
const MAX_REPORTED: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Turn {
    /// The agent's session ID, as it gave it.
    pub session: String,
    /// The working tree when the turn began, as a Git tree.
    pub tree: String,
    pub started_at: u64,
    /// Stops of this turn that JevGate blocked.
    #[serde(default)]
    pub blocks: u32,
    /// The first line of the last block's reason. Gemini CLI and Cursor send
    /// the reason back as the next prompt, which continues this turn.
    #[serde(default)]
    pub block_line: Option<String>,
    /// The working tree at the last block: a stop that finds it unchanged
    /// does not block again, since the agent kept the code as it was.
    #[serde(default)]
    pub blocked_tree: Option<String>,
    /// Why a stop was not checked, for the next event that can tell the agent.
    #[serde(default)]
    pub notice: Option<String>,
    /// Its stop could not be checked (an outage, a 402, the time ran out),
    /// so the next turn begins where this one did and its stop checks both.
    #[serde(default)]
    pub unchecked: bool,
    /// It began where an unchecked turn did, so its changes go back to then.
    #[serde(default)]
    pub carried: bool,
    /// Fingerprints of the findings, and ids of the guards, already given to
    /// the agent after an edit this turn: a file edited ten times would
    /// otherwise repeat them ten times.
    #[serde(default)]
    pub reported: Vec<String>,
}

impl Turn {
    /// A turn that begins with the working tree `tree`.
    pub fn begin(session: &str, tree: String, notice: Option<String>) -> Self {
        Self {
            session: session.to_string(),
            tree,
            started_at: schema::now(),
            notice,
            ..Self::default()
        }
    }

    /// A turn that begins where `unchecked`, a turn whose stop could not be
    /// checked, began, owing the agent its notice.
    pub fn carry(session: &str, unchecked: Turn) -> Self {
        Self {
            carried: true,
            ..Self::begin(session, unchecked.tree, unchecked.notice)
        }
    }

    /// Whether `prompt` is this turn's own block reason sent back as a prompt.
    pub fn continued_by(&self, prompt: &str) -> bool {
        self.block_line
            .as_ref()
            .is_some_and(|line| prompt.contains(line.as_str()))
    }

    /// Remember that findings and guards with these ids (fingerprints)
    /// reached the agent; whether any was new. The oldest are forgotten past
    /// [`MAX_REPORTED`], so the turn file stays small, and a forgotten one is
    /// only told again.
    pub fn report<'a>(&mut self, given: impl IntoIterator<Item = &'a str>) -> bool {
        let before = self.reported.len();
        for id in given {
            if !id.is_empty() && !self.reported.iter().any(|r| r == id) {
                self.reported.push(id.to_string());
            }
        }
        let excess = self.reported.len().saturating_sub(MAX_REPORTED);
        self.reported.drain(..excess);
        self.reported.len() != before || excess > 0
    }
}

/// `.jevgate/turns/` under `root`, created when missing.
pub(super) fn directory(root: &Path) -> Result<PathBuf> {
    let directory = storage::state_directory(root)?.join("turns");
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

/// The turn file of `session`: named by a hash, so any session ID is a safe name.
fn file_name(session: &str) -> String {
    schema::hash(session.as_bytes())[..32].to_string()
}

/// The session's turn, if one was recorded, is readable and its start is
/// still in the repository: a turn whose tree Git pruned counts as none, so
/// the next stop records a new start instead of failing on it every time.
pub(super) fn load(root: &Path, session: &str) -> Option<Turn> {
    let path = root
        .join(".jevgate/turns")
        .join(format!("{}.json", file_name(session)));
    let text = crate::inventory::read_source(&path, MAX_BYTES).ok()?;
    let turn: Turn = serde_json::from_str(&text).ok()?;
    (turn.session == session && revision::has_tree(root, &turn.tree)).then_some(turn)
}

pub(super) fn save(root: &Path, turn: &Turn) -> Result<()> {
    let path = directory(root)?.join(format!("{}.json", file_name(&turn.session)));
    storage::atomic(&path, &serde_json::to_vec(turn)?)
}

/// The working tree now, as a Git tree, starting from the repository's Git
/// `index` and finished by `deadline`; the scratch copy is named for the
/// session and the process, so hooks of parallel edits never share one.
pub(super) fn snapshot(
    root: &Path,
    index: &Path,
    session: &str,
    deadline: Instant,
) -> Result<String> {
    let scratch = directory(root)?.join(format!(
        "{}.{}.index",
        file_name(session),
        std::process::id()
    ));
    revision::snapshot(root, index, &scratch, deadline)
        .context("Cannot take a snapshot of the working tree")
}

/// How long a mark of an event holds: an agent that runs two copies of
/// JevGate's hooks (Cursor running Claude Code's beside its own, or Claude
/// Code the plugin's beside the settings') starts both at once, and each
/// gave the agent the same findings and blocked its stop, the second as
/// "block 1 of 3" again. A mark older than this was left by a process that
/// was stopped before it removed it.
const TWIN_SECS: u64 = 5;

/// The event this process answers, marked while it does: an identical
/// event another `jevgate hook` starts meanwhile is that event sent twice.
pub(super) struct Claim(Option<PathBuf>);

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(mark) = &self.0 {
            let _ = std::fs::remove_file(mark);
        }
    }
}

/// This process's claim on the event `key` names; none when another process
/// is answering it now. A mark created exclusively decides between two
/// processes started together, and goes when the answer is written, so the
/// same event sent again later is answered again.
pub(super) fn claim(root: &Path, key: &str) -> Option<Claim> {
    let Ok(directory) = directory(root) else {
        return Some(Claim(None));
    };
    let mark = directory.join(format!("{}.event", file_name(key)));
    let create = || {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&mark)
    };
    match create() {
        Ok(_) => Some(Claim(Some(mark))),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if age(&mark).is_some_and(|age| age.as_secs() < TWIN_SECS) {
                return None;
            }
            let _ = std::fs::remove_file(&mark);
            Some(Claim(create().ok().map(|_| mark)))
        }
        Err(_) => Some(Claim(None)),
    }
}

/// How long ago `path` was last written.
fn age(path: &Path) -> Option<std::time::Duration> {
    std::fs::symlink_metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
}

/// Remove the files of `directory` idle for a week: turn files and scratch
/// indexes a killed hook left under `.jevgate/turns/`, or marks outside Git.
/// A `directory` that is a symbolic link is not read, and only regular
/// files are removed, so nothing is removed through a link.
pub(super) fn prune(directory: &Path) {
    if directory.is_symlink() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file = entry.file_type().is_ok_and(|kind| kind.is_file());
        if file && age(&path).is_some_and(|age| age.as_secs() > KEPT_SECS) {
            let _ = std::fs::remove_file(path);
        }
    }
}
