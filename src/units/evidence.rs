//! Request building shared by every planner: one file's facts, the request
//! envelope, packing, and stable identities.
use super::{Asked, Questions};
use crate::{schema::Location, token_budget::TokenBudget};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

/// Packed requests stay well below the provider's state limit so each
/// question sees a small state (about six thousand tokens). The budget is in
/// bytes, not calibrated tokens, so packing and cache identity stay stable.
const PACK_BYTES: usize = 18_000;

/// Shared facts one file's planners need.
pub(super) struct FileContext<'a> {
    pub owner: usize,
    pub path: &'a Path,
    pub language: &'static str,
    pub source: &'a str,
    pub source_hash: &'a str,
    pub model: &'a str,
    pub budget: &'a TokenBudget,
}

impl FileContext<'_> {
    pub(super) fn location(
        &self,
        start_line: usize,
        end_line: usize,
        symbol: Option<&str>,
    ) -> Location {
        Location {
            path: self.path.to_path_buf(),
            start_line,
            end_line,
            symbol: symbol.map(str::to_string),
        }
    }

    pub(super) fn file_state(&self) -> Value {
        json!({"path": self.path, "language": self.language})
    }

    pub(super) fn request(
        &self,
        stage: &str,
        state: Value,
        questions: Questions,
    ) -> (Value, Asked) {
        request(
            self.model,
            stage,
            &[(self.path, self.source_hash)],
            state,
            questions,
        )
    }
}

/// Freshness hashes and the stage stay in local metadata; only model, state
/// and questions are uploaded.
pub(super) fn request(
    model: &str,
    stage: &str,
    sources: &[(&Path, &str)],
    state: Value,
    questions: Questions,
) -> (Value, Asked) {
    let (questions, asked) = questions.finish();
    let sources: Vec<_> = sources
        .iter()
        .map(|(path, hash)| json!({"path": path, "source_hash": hash}))
        .collect();
    let request = json!({
        "model": model,
        "state": state,
        "questions": questions,
        "jevgate": {"stage": stage, "sources": sources},
    });
    (request, asked)
}

/// Greedy packing in order: at most `limit` items and `PACK_BYTES` of state.
pub(super) fn pack<T>(items: Vec<T>, limit: usize, state: impl Fn(&T) -> &Value) -> Vec<Vec<T>> {
    let mut packs: Vec<Vec<T>> = Vec::new();
    let mut used = 0;
    for item in items {
        let size = serde_json::to_vec(state(&item)).map_or(0, |v| v.len());
        match packs.last_mut() {
            Some(pack) if pack.len() < limit && used + size <= PACK_BYTES => {
                used += size;
                pack.push(item);
            }
            _ => {
                used = size;
                packs.push(vec![item]);
            }
        }
    }
    packs
}

pub(super) fn identity(parts: &[&str]) -> String {
    crate::schema::hash(parts.join("\u{0}").as_bytes())
}

pub(super) fn compact(text: &str) -> String {
    text.split_whitespace().collect()
}

/// Unique ids for units that share a name in one file.
pub(super) fn unique_ids<'a>(prefix: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeMap::<String, usize>::new();
    names
        .map(|name| {
            let count = seen.entry(name.to_string()).or_default();
            *count += 1;
            if *count == 1 {
                format!("{prefix}:{name}")
            } else {
                format!("{prefix}:{name}#{count}")
            }
        })
        .collect()
}
