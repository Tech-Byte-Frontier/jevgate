//! Evidence units: small typed requests about one function group, one file
//! outline, one candidate pair or a few tests. Code builds the evidence,
//! Jev answers short literal questions, and `compose` turns answers into results.
pub mod compose;
mod duplicates;
mod functions;
mod outline;
pub mod questions;
mod test_units;

use crate::{
    analysis::{
        clones::{self, SourceFile},
        test_map,
        units::{self as parsed, FileUnits, Unit},
    },
    catalog,
    file_kind::View,
    inventory::Input,
    options::CheckArgs,
    requests::TokenBudget,
    schema::{Answer, FileResult, Judgment, Location, Pass},
};
use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
};

/// Packed requests stay well below the provider's state limit so each
/// question sees a small state (about six thousand tokens). The budget is in
/// bytes, not calibrated tokens, so packing and cache identity stay stable.
const PACK_BYTES: usize = 18_000;
const PACK_ITEMS: usize = 8;

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
        differences: Vec<clones::Difference>,
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

/// A scope of files sharing clone, subject and caller evidence.
struct Scope<'a> {
    owners: Vec<usize>,
    inputs: &'a [Input],
    views: &'a BTreeMap<usize, View>,
    units: BTreeMap<usize, FileUnits>,
    context: Vec<(PathBuf, &'a str, FileUnits)>,
}

impl Scope<'_> {
    fn test_lines(&self, owner: usize) -> Vec<Range<usize>> {
        self.views[&owner]
            .test_lines
            .iter()
            .map(|r| r.start_line..r.end_line + 1)
            .collect()
    }

    /// Callable units outside tests, in selected files and explicit context.
    fn scope_units(&self) -> impl Iterator<Item = (&Path, &Unit)> {
        let selected = self.owners.iter().flat_map(move |owner| {
            let lines = self.test_lines(*owner);
            let path = self.inputs[*owner].result.path.as_path();
            self.units[owner]
                .units
                .iter()
                .filter(move |u| u.callable() && !lines.iter().any(|l| u.overlaps(l)))
                .map(move |u| (path, u))
        });
        let context = self.context.iter().flat_map(|(path, _, units)| {
            units
                .units
                .iter()
                .filter(|u| u.callable())
                .map(move |u| (path.as_path(), u))
        });
        selected.chain(context)
    }
}

pub fn plan(
    inputs: &[Input],
    views: &BTreeMap<usize, View>,
    args: &CheckArgs,
    budget: &TokenBudget,
) -> Plan {
    let mut result = Plan::default();
    let mut scope = Scope {
        owners: Vec::new(),
        inputs,
        views,
        units: BTreeMap::new(),
        context: Vec::new(),
    };
    for (&owner, view) in views {
        let input = &inputs[owner];
        let source = input.source.as_deref().unwrap_or("");
        match parsed::parse(&input.result.path, source) {
            Ok(units) if units.parsed => {
                scope.units.insert(owner, units);
                scope.owners.push(owner);
            }
            Ok(_) => {
                result.skipped.insert(
                    owner,
                    format!(
                        "No {} parser; units cannot be located, so this file was not judged.",
                        view.classification.language
                    ),
                );
            }
            Err(_) => {
                result
                    .skipped
                    .insert(owner, "Syntax errors; this file was not judged.".into());
            }
        }
    }
    if let Some(first) = scope.owners.first() {
        for context in &inputs[*first].context {
            let units = parsed::parse(&context.file.path, &context.source).unwrap_or_default();
            scope
                .context
                .push((context.file.path.clone(), context.source.as_str(), units));
        }
    }
    let enabled = |key: &str| args.rules.iter().any(|r| r == key || r == catalog::id(key));
    let pairs = if enabled(catalog::SHARED_LOGIC) {
        duplicate_candidates(&scope)
    } else {
        clones::Candidates::default()
    };
    let callers = callers(&scope);
    let mut subjects = BTreeMap::<String, String>::new();
    for (_, unit) in scope.scope_units() {
        subjects
            .entry(unit.short_name.clone())
            .or_insert_with(|| unit.signature.clone());
    }
    let mut hashes: BTreeMap<PathBuf, String> = scope
        .owners
        .iter()
        .map(|&o| {
            (
                inputs[o].result.path.clone(),
                inputs[o].result.source_hash.clone(),
            )
        })
        .collect();
    if let Some(first) = scope.owners.first() {
        for context in &inputs[*first].context {
            hashes.insert(context.file.path.clone(), context.file.source_hash.clone());
        }
    }
    for &owner in &scope.owners {
        let input = &inputs[owner];
        let view = &views[&owner];
        let context = FileContext {
            owner,
            path: &input.result.path,
            language: crate::file_kind::language(&input.result.path),
            source: input.source.as_deref().unwrap_or(""),
            source_hash: &input.result.source_hash,
            model: &args.model,
        };
        let mut file = FilePlan {
            path: input.result.path.clone(),
            ..Default::default()
        };
        let lines = scope.test_lines(owner);
        let units = &scope.units[&owner].units;
        let in_tests = |u: &Unit| lines.iter().any(|l| u.overlaps(l));
        let cases = if view.tests {
            test_map::cases(context.path, context.source)
                .unwrap_or_default()
                .into_iter()
                .filter(|c| lines.iter().any(|l| l.contains(&c.line)))
                .collect()
        } else {
            Vec::new()
        };
        if enabled(catalog::FUNCTION_SIMPLIFICATION) {
            let judged: Vec<&Unit> = units
                .iter()
                .filter(|u| u.callable())
                .filter(|u| {
                    if in_tests(u) {
                        view.tests
                            && !cases
                                .iter()
                                .any(|c| c.line <= u.line && u.end_line <= c.end_line)
                    } else {
                        view.application
                    }
                })
                .collect();
            if view.application || !judged.is_empty() {
                file.rules.insert(catalog::FUNCTION_SIMPLIFICATION, 0);
                functions::plan(
                    &context,
                    &judged,
                    &scope,
                    budget,
                    &mut file,
                    &mut result.requests,
                );
            }
        }
        if enabled(catalog::FILE_ORGANIZATION) && view.application {
            let members: Vec<usize> = (0..units.len()).filter(|&i| !in_tests(&units[i])).collect();
            file.rules.insert(catalog::FILE_ORGANIZATION, 0);
            if members.len() >= 2 {
                outline::plan(
                    &context,
                    &scope.units[&owner],
                    &members,
                    &callers,
                    budget,
                    &mut file,
                    &mut result.requests,
                );
            }
        }
        if enabled(catalog::SHARED_LOGIC) {
            file.rules.insert(catalog::SHARED_LOGIC, 0);
            duplicates::plan(
                &context,
                &pairs,
                &hashes,
                budget,
                &mut file,
                &mut result.requests,
            );
        }
        if view.tests && args.include_tests {
            let mut cases = cases;
            test_map::link(&mut cases, &subjects.keys().cloned().collect());
            if enabled(catalog::TEST_VALUE) {
                file.rules.insert(catalog::TEST_VALUE, 0);
                test_units::plan_values(
                    &context,
                    &cases,
                    &subjects,
                    budget,
                    &mut file,
                    &mut result.requests,
                );
            }
            if enabled(catalog::TEST_REDUNDANCY) {
                file.rules.insert(catalog::TEST_REDUNDANCY, 0);
                test_units::plan_pairs(
                    &context,
                    &cases,
                    &subjects,
                    budget,
                    &mut file,
                    &mut result.requests,
                );
            }
        }
        result.files.insert(owner, file);
    }
    result
}

fn duplicate_candidates(scope: &Scope<'_>) -> clones::Candidates {
    let lines: BTreeMap<usize, Vec<Range<usize>>> = scope
        .owners
        .iter()
        .map(|&owner| {
            let view = &scope.views[&owner];
            (
                owner,
                if view.tests {
                    Vec::new()
                } else {
                    scope.test_lines(owner)
                },
            )
        })
        .collect();
    let mut files: Vec<SourceFile<'_>> = scope
        .owners
        .iter()
        .map(|owner| SourceFile {
            path: &scope.inputs[*owner].result.path,
            source: scope.inputs[*owner].source.as_deref().unwrap_or(""),
            selected: true,
            units: &scope.units[owner].units,
            excluded: &lines[owner],
        })
        .collect();
    for (path, source, units) in &scope.context {
        files.push(SourceFile {
            path,
            source,
            selected: false,
            units: &units.units,
            excluded: &[],
        });
    }
    clones::find(&files)
}

/// Short callee name to the other selected files that call it.
fn callers(scope: &Scope<'_>) -> BTreeMap<String, BTreeSet<PathBuf>> {
    let mut callers = BTreeMap::<String, BTreeSet<PathBuf>>::new();
    for &owner in &scope.owners {
        for unit in &scope.units[&owner].units {
            for call in &unit.calls {
                callers
                    .entry(call.clone())
                    .or_default()
                    .insert(scope.inputs[owner].result.path.clone());
            }
        }
    }
    callers
}

/// Greedy packing in order: at most `PACK_ITEMS` items and `PACK_BYTES` of state.
fn pack<T>(items: Vec<T>, state: impl Fn(&T) -> &Value) -> Vec<Vec<T>> {
    let mut packs: Vec<Vec<T>> = Vec::new();
    let mut used = 0;
    for item in items {
        let size = serde_json::to_vec(state(&item)).map_or(0, |v| v.len());
        match packs.last_mut() {
            Some(pack) if pack.len() < PACK_ITEMS && used + size <= PACK_BYTES => {
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
