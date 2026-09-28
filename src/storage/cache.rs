//! Cached answers under `.jevgate/cache/`. Each answer is kept by the state it
//! is about and its question: one file per state in `answers/`, holding every
//! question's answer. The whole-request entries of earlier versions, one file
//! per request directly in `cache/`, are read and kept, never written.
//!
//! One file per state rather than per question: JevGate's own `--rule all`
//! first pass holds 21,138 questions in 2,974 requests about 2,702 states,
//! every file takes a 4 KiB block, and every save waits for the disk (4.8 ms
//! a file on macOS). One file per state keeps the file count and the reads of
//! one file per request.
use super::{Durability, Store, atomic};
use crate::schema::now;
use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// A cache file larger than this is ignored, and what it held is asked again.
const CACHE_ENTRY_BYTES: u64 = 1_048_576;

/// The answer to one question about one state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedAnswer {
    pub created_at: u64,
    /// The model version that answered.
    pub model: String,
    /// The typed answer's fields.
    pub answer: Value,
    /// Its share of the usage of the request that asked it.
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// The answers about one state, by question key.
#[derive(Serialize, Deserialize)]
struct StateEntry {
    state_hash: String,
    answers: BTreeMap<String, CachedAnswer>,
}

/// A whole request's answer, under the hash of the request: read, never written.
#[derive(Serialize, Deserialize)]
struct RequestEntry {
    request_hash: String,
    created_at: u64,
    response: Value,
}

pub(super) fn answers_directory(directory: &Path) -> PathBuf {
    directory.join("cache").join("answers")
}

fn state_path(directory: &Path, state: &str) -> PathBuf {
    answers_directory(directory).join(format!("{state}.json"))
}

/// A cache file's JSON; none when it is missing, a symlink, too large or not
/// what it should be.
fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&crate::inventory::read_source(path, CACHE_ENTRY_BYTES).ok()?).ok()
}

/// Whether an answer given at `created_at` is still used: always for a pinned
/// model (`ttl` is none), for `ttl` seconds for an alias, and never when its
/// time is in the future.
fn current(created_at: u64, ttl: Option<u64>) -> bool {
    now()
        .checked_sub(created_at)
        .is_some_and(|age| ttl.is_none_or(|ttl| age < ttl))
}

/// Reads cached answers: through the open store in a run, or without its
/// lock while planning and in dry runs, which write nothing.
pub struct CacheReader {
    directory: PathBuf,
}

impl CacheReader {
    /// The cache under `root`, read without opening the store: no lock is
    /// taken and no directory is created. None when `.jevgate` is not a real
    /// directory.
    pub fn peek(root: &Path) -> Option<Self> {
        let directory = root.join(".jevgate");
        (!directory.is_symlink() && directory.is_dir()).then_some(Self { directory })
    }

    /// The answers about `state` by question key, without those `ttl` has
    /// expired or that claim a time in the future.
    pub fn answers(&self, state: &str, ttl: Option<u64>) -> BTreeMap<String, CachedAnswer> {
        let mut answers = read_json::<StateEntry>(&state_path(&self.directory, state))
            .filter(|entry| entry.state_hash == state)
            .map(|entry| entry.answers)
            .unwrap_or_default();
        answers.retain(|_, answer| current(answer.created_at, ttl));
        answers
    }

    /// The response a version before per-question caching kept for the whole
    /// request `hash`, and when it was given.
    pub fn request(&self, hash: &str, ttl: Option<u64>) -> Option<(Value, u64)> {
        let path = self.directory.join("cache").join(format!("{hash}.json"));
        let entry = read_json::<RequestEntry>(&path)?;
        (entry.request_hash == hash && current(entry.created_at, ttl))
            .then_some((entry.response, entry.created_at))
    }
}

impl Store {
    pub fn reader(&self) -> CacheReader {
        CacheReader {
            directory: self.directory.clone(),
        }
    }

    /// Keep answers the provider gave about `state`, by question key.
    pub fn save_answers(&self, state: &str, answers: BTreeMap<String, CachedAnswer>) -> Result<()> {
        self.merge_answers(state, answers, Durability::Synced)
    }

    /// Keep answers about `state` copied from a whole-request entry. That
    /// entry stays on disk, so a copy a crash loses is made again on the
    /// next run, and the write does not wait for the disk: the first run
    /// after upgrading copies every cached request once, which took 4.8 ms
    /// a file on macOS when synced, or about 14 s for JevGate's own cache.
    pub fn copy_answers(&self, state: &str, answers: BTreeMap<String, CachedAnswer>) -> Result<()> {
        self.merge_answers(state, answers, Durability::Unsynced)
    }

    /// Add `answers` to their state's file, replacing earlier answers to the
    /// same questions. A file that would grow past the size bound keeps only
    /// these answers, so it stays readable.
    fn merge_answers(
        &self,
        state: &str,
        answers: BTreeMap<String, CachedAnswer>,
        durability: Durability,
    ) -> Result<()> {
        let path = state_path(&self.directory, state);
        let mut entry = read_json::<StateEntry>(&path)
            .filter(|entry| entry.state_hash == state)
            .unwrap_or_else(|| StateEntry {
                state_hash: state.into(),
                answers: BTreeMap::new(),
            });
        let saved: BTreeSet<String> = answers.keys().cloned().collect();
        entry.answers.extend(answers);
        let mut bytes = serde_json::to_vec(&entry)?;
        if bytes.len() as u64 > CACHE_ENTRY_BYTES {
            entry.answers.retain(|key, _| saved.contains(key));
            bytes = serde_json::to_vec(&entry)?;
        }
        atomic(&path, &bytes, durability)
    }

    /// Write a whole-request entry, to test reading them.
    #[cfg(test)]
    pub fn save_request(&self, hash: &str, response: &Value, created_at: u64) -> Result<()> {
        let entry = RequestEntry {
            request_hash: hash.into(),
            response: response.clone(),
            created_at,
        };
        atomic(
            &self.directory.join("cache").join(format!("{hash}.json")),
            &serde_json::to_vec(&entry)?,
            Durability::Synced,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn answer(padding: usize) -> CachedAnswer {
        CachedAnswer {
            created_at: now(),
            model: "jev-1.13.0".into(),
            answer: json!({"type": "noul", "noul": 0.5, "padding": "x".repeat(padding)}),
            input_tokens: 1,
            output_tokens: 0,
        }
    }

    #[test]
    fn a_state_file_keeps_its_answers_within_the_size_bound() {
        let project = crate::tests::Project::new();
        let store = Store::open(&project.0).unwrap();
        let keys = |store: &Store| {
            let answers = store.reader().answers("state", None);
            answers.into_keys().collect::<Vec<_>>()
        };
        store
            .save_answers("state", [("a".into(), answer(600_000))].into())
            .unwrap();
        store
            .save_answers("state", [("b".into(), answer(600_000))].into())
            .unwrap();
        assert_eq!(keys(&store), ["b"], "together they would pass the bound");
        store
            .copy_answers("state", [("c".into(), answer(10))].into())
            .unwrap();
        assert_eq!(keys(&store), ["b", "c"]);
        assert!(store.reader().answers("other", None).is_empty());
    }
}
