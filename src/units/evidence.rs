//! Request building shared by every planner: one file's facts, the request
//! envelope, packing, and stable identities.
use super::{Asked, PACK_ITEMS, Questions};
use crate::{schema::Location, token_budget::TokenBudget};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
    /// What a web framework makes of the file, such as a Next.js route
    /// handler or Server Actions module, sent beside its path.
    pub framework: Option<String>,
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

    /// The file's path and language, for questions about how code reads:
    /// a framework role sent there moved split answers without informing them.
    pub(super) fn plain_state(&self) -> Value {
        json!({"path": self.path, "language": self.language})
    }

    /// The file's path, language and framework role, for questions about
    /// where values come from and go, and which values a reader must guess.
    pub(super) fn file_state(&self) -> Value {
        match &self.framework {
            Some(framework) => {
                json!({"path": self.path, "language": self.language, "framework": framework})
            }
            None => json!({"path": self.path, "language": self.language}),
        }
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
            questions.reworded(self.language),
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
    let (mut questions, asked) = questions.finish();
    if state["file"]["framework"].is_string() {
        point_to_framework(&mut questions);
    }
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

/// What a note adds when the state names the file's framework role: stated
/// only in the state, a client component's role did not clear its browser
/// requests, since the questions never pointed at it.
const FRAMEWORK_NOTE: &str =
    "`file.framework` states who calls this file's code and where it runs.";

fn point_to_framework(questions: &mut serde_json::Map<String, Value>) {
    for body in questions.values_mut() {
        let instructions = &mut body["instructions"];
        let note = match instructions["note"].as_str() {
            Some(note) => format!("{FRAMEWORK_NOTE} {note}"),
            None => FRAMEWORK_NOTE.to_string(),
        };
        instructions["note"] = Value::String(note);
    }
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

/// One in this many keys ends a run of items packed together.
const RUN_ENDS: u8 = 4;

/// Packing within runs of consecutive items: a run ends after the last item
/// of a key (a definition's name, a section's heading) whose SHA-256 first
/// byte is a multiple of `RUN_ENDS`, so where runs end depends on keys
/// rather than positions, and an item added, removed or resized re-packs
/// only its own run. Packed greedily in file order, one such edit shifted
/// every later pack of the file and none of them hit the cache; packing
/// each key alone kept the others too, but doubled to quadrupled requests.
pub(super) fn pack_runs<T>(
    items: Vec<T>,
    key: impl Fn(&T) -> &str,
    state: impl Fn(&T) -> &Value,
) -> Vec<Vec<T>> {
    let ends: Vec<bool> = items
        .iter()
        .enumerate()
        .map(|(at, item)| {
            let own = key(item);
            let last = items.get(at + 1).is_none_or(|next| key(next) != own);
            last && Sha256::digest(own.as_bytes())[0] % RUN_ENDS == 0
        })
        .collect();
    let mut packs = Vec::new();
    let mut run = Vec::new();
    for (item, end) in items.into_iter().zip(ends) {
        run.push(item);
        if end {
            packs.extend(pack(std::mem::take(&mut run), PACK_ITEMS, &state));
        }
    }
    packs.extend(pack(run, PACK_ITEMS, &state));
    packs
}

pub(super) fn identity(parts: &[&str]) -> String {
    crate::schema::hash(parts.join(crate::schema::HASH_SEPARATOR).as_bytes())
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
