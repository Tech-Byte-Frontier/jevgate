//! Evidence units: small typed requests about one function group, one file
//! outline, one candidate pair or a few tests. Code builds the evidence,
//! Jev answers short literal questions, and `compose` turns answers into results.
pub mod compose;
mod duplicates;
mod functions;
pub(crate) mod outline;
mod plan;
pub mod questions;
mod test_units;
mod wording;

use plan::Scope;
pub use plan::plan;

use crate::schema::{Answer, FileResult, Judgment, Location, Pass};
use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Packed requests stay well below the provider's state limit so each
/// question sees a small state (about six thousand tokens). The budget is in
/// bytes, not calibrated tokens, so packing and cache identity stay stable.
const PACK_BYTES: usize = 18_000;
const PACK_ITEMS: usize = 8;
/// Tests are sent one per request: seven unrelated tests in the same state
/// left about 40% more test-value questions undecided.
const TEST_PACK_ITEMS: usize = 1;

/// The questions one request asks, mapped back to units.
#[derive(Clone, Debug, Default)]
pub struct Asked {
    pub questions: Vec<AskedQuestion>,
}

#[derive(Clone, Debug)]
pub struct AskedQuestion {
    pub key: String,
    pub rule: &'static str,
    pub unit: String,
    pub question: &'static str,
    pub pass: Pass,
}

impl Asked {
    #[allow(clippy::too_many_arguments)]
    fn ask(
        &mut self,
        questions: &mut Map<String, Value>,
        key: String,
        body: Value,
        unit: &str,
        rule: &'static str,
        question: &'static str,
        pass: Pass,
    ) {
        questions.insert(key.clone(), body);
        self.questions.push(AskedQuestion {
            key,
            rule,
            unit: unit.into(),
            question,
            pass,
        });
    }
}

pub struct Planned {
    pub owner: usize,
    pub request: Value,
    pub asked: Asked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Judged,
    /// Below the minimum body size; never clear.
    TooSmall,
    /// The unit alone exceeds the provider limit, so it was not sent.
    NeedsContext,
}

#[derive(Clone, Debug)]
pub struct GroupInfo {
    pub id: String,
    pub names: Vec<String>,
    pub locations: Vec<Location>,
}

#[derive(Clone, Debug)]
pub enum Detail {
    Function,
    Outline {
        groups: Vec<GroupInfo>,
    },
    Pair {
        differences: Vec<crate::analysis::clones::Difference>,
        /// Both copies sit in one test case: the remedy is a table of cases,
        /// not a shared implementation.
        within_test: bool,
        /// The owning copy is test code: shared steps belong in a fixture or helper.
        in_tests: bool,
    },
    Test,
    TestPair {
        names: [String; 2],
        subject: String,
    },
}

#[derive(Clone, Debug)]
pub struct UnitPlan {
    pub rule: &'static str,
    /// Unique within the file; judgments refer to it.
    pub id: String,
    pub name: String,
    pub presence: Presence,
    pub locations: Vec<Location>,
    pub quote: Option<String>,
    pub lines: usize,
    /// Identity for the finding fingerprint: survives moves and unrelated edits.
    pub identity: String,
    pub detail: Detail,
    pub recheck: Option<(Value, Asked)>,
}

#[derive(Clone, Debug, Default)]
pub struct FilePlan {
    pub path: PathBuf,
    /// Rules that apply to this file, with the candidates omitted by caps.
    pub rules: BTreeMap<&'static str, usize>,
    pub units: Vec<UnitPlan>,
}

#[derive(Default)]
pub struct Plan {
    pub files: BTreeMap<usize, FilePlan>,
    pub requests: Vec<Planned>,
    /// Files with no supported parser or with syntax errors, and why.
    pub skipped: BTreeMap<usize, String>,
}

/// Shared facts one file's planners need.
struct FileContext<'a> {
    owner: usize,
    path: &'a Path,
    language: &'static str,
    source: &'a str,
    source_hash: &'a str,
    model: &'a str,
}

impl FileContext<'_> {
    fn location(&self, start_line: usize, end_line: usize, symbol: Option<&str>) -> Location {
        Location {
            path: self.path.to_path_buf(),
            start_line,
            end_line,
            symbol: symbol.map(str::to_string),
        }
    }

    fn file_state(&self) -> Value {
        json!({"path": self.path, "language": self.language})
    }

    fn request(&self, stage: &str, state: Value, questions: Map<String, Value>) -> Value {
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
fn request(
    model: &str,
    stage: &str,
    sources: &[(&Path, &str)],
    state: Value,
    questions: Map<String, Value>,
) -> Value {
    let sources: Vec<_> = sources
        .iter()
        .map(|(path, hash)| json!({"path": path, "source_hash": hash}))
        .collect();
    json!({
        "model": model,
        "state": state,
        "questions": questions,
        "jevgate": {"stage": stage, "sources": sources},
    })
}

/// Greedy packing in order: at most `limit` items and `PACK_BYTES` of state.
fn pack<T>(items: Vec<T>, limit: usize, state: impl Fn(&T) -> &Value) -> Vec<Vec<T>> {
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

fn identity(parts: &[&str]) -> String {
    crate::schema::hash(parts.join("\u{0}").as_bytes())
}

fn compact(text: &str) -> String {
    text.split_whitespace().collect()
}

/// Unique ids for units that share a name in one file.
fn unique_ids<'a>(prefix: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
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

/// Record the answers of one request as typed judgments on its owner file.
pub fn record(file: &mut FileResult, asked: &Asked, body: &Value) -> Result<()> {
    file.model = body["model"].as_str().map(str::to_owned);
    for question in &asked.questions {
        let answer: Answer = serde_json::from_value(body["answers"][&question.key].clone())
            .with_context(|| format!("Invalid answer for {}", question.key))?;
        file.judgments.retain(|j| {
            !(j.unit == question.unit && j.question == question.question && j.pass == question.pass)
        });
        file.judgments.push(Judgment {
            rule: question.rule.into(),
            unit: question.unit.clone(),
            question: question.question.into(),
            version: questions::VERSION.into(),
            pass: question.pass,
            answer,
        });
    }
    Ok(())
}

/// One recheck per unit that stayed uncertain after the first pass.
pub fn rechecks(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    let mut planned = Vec::new();
    for (&owner, file_plan) in &plan.files {
        let file = &files[owner];
        if file.status == crate::schema::Status::Error {
            continue;
        }
        let uncertain = compose::uncertain_units(file_plan, &file.judgments);
        for unit in &file_plan.units {
            if let Some((request, asked)) = &unit.recheck
                && uncertain.contains(&unit.id)
            {
                planned.push(Planned {
                    owner,
                    request: request.clone(),
                    asked: asked.clone(),
                });
            }
        }
    }
    planned
}

#[cfg(test)]
mod tests;
