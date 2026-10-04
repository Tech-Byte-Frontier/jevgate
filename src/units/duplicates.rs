//! Shared logic: one request per candidate group, asking the look-here
//! question over every copy: whether the snippets repeat one piece of logic
//! a maintainer should keep in one place. A coding agent verifies each flag.
use super::{
    Asked, Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan, identity,
    outcome::LOOK, questions, request,
};
use crate::{
    analysis::clones::{Candidates, Pair, Site},
    catalog::SHARED_LOGIC,
    schema::{Location, Pass},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

/// Copies a request shows at most: the two it was found from first.
const SHOWN_SITES: usize = 6;

pub(super) fn plan(
    file: &FileContext<'_>,
    candidates: &Candidates,
    hashes: &BTreeMap<PathBuf, String>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    if let Some(omitted) = candidates.omitted.get(file.path) {
        out.rules.insert(SHARED_LOGIC, *omitted);
    }
    for pair in candidates.pairs.iter().filter(|p| p.a.path == file.path) {
        let id = format!(
            "pair:{}:{}:{}",
            pair.a.start_line,
            pair.b.path.display(),
            pair.b.start_line
        );
        let sent = file.push_fitting(build(file, pair, hashes, &id), requests);
        let presence = Presence::judged_if(sent);
        let members = members(pair);
        out.units.push(UnitPlan {
            rule: SHARED_LOGIC,
            name: match pair.copies.len() {
                // Two places in one function: `search` (…:26) and `search` (…:32) read as two functions.
                0 if pair.a.path == pair.b.path
                    && pair.a.function.is_some()
                    && pair.a.function == pair.b.function =>
                {
                    format!(
                        "Lines {} and {} of `{}` ({})",
                        pair.a.start_line,
                        pair.b.start_line,
                        pair.a.function.as_deref().unwrap_or(""),
                        pair.a.path.display()
                    )
                }
                0 => format!("{} and {}", site_label(&pair.a), site_label(&pair.b)),
                n => format!(
                    "{}, {} and {n} more cop{}",
                    site_label(&pair.a),
                    site_label(&pair.b),
                    if n == 1 { "y" } else { "ies" }
                ),
            },
            id,
            presence,
            locations: sites(pair).map(location).collect(),
            quote: Some(pair.a.quote.clone()),
            lines: pair.a.end_line + 1 - pair.a.start_line,
            identity: identity(&members.iter().map(String::as_str).collect::<Vec<_>>()),
            detail: Detail::Pair {
                members,
                v1: identity(&[
                    pair.a.function.as_deref().unwrap_or(""),
                    &pair.b.path.to_string_lossy(),
                    pair.b.function.as_deref().unwrap_or(""),
                    &pair.normalized,
                ]),
            },
            recheck: None,
        });
    }
}

/// The copies a pair's finding is identified by, in order: each function
/// as `path::function`, whichever window of it repeats and whichever pair
/// represents the group, and a copy outside a function as `path#hash` of
/// the repeated statements. Neither the file that owns the finding nor
/// which files a check selected changes them.
fn members(pair: &Pair) -> Vec<String> {
    let mut members: Vec<String> = sites(pair)
        .map(|site| match &site.function {
            Some(function) => format!("{}::{function}", site.path.display()),
            None => format!("{}#{}", site.path.display(), pair.normalized),
        })
        .collect();
    members.sort();
    members.dedup();
    members
}

/// A pair's copies: the two it was found from, then the group's others.
fn sites(pair: &Pair) -> impl Iterator<Item = &Site> {
    [&pair.a, &pair.b].into_iter().chain(&pair.copies)
}

fn site_label(site: &Site) -> String {
    match &site.function {
        Some(function) => format!("`{function}` ({}:{})", site.path.display(), site.start_line),
        None => format!("{}:{}", site.path.display(), site.start_line),
    }
}

fn location(site: &Site) -> Location {
    Location {
        path: site.path.clone(),
        start_line: site.start_line,
        end_line: site.end_line,
        symbol: site.function.clone(),
    }
}

fn site_state(site: &Site) -> Value {
    let mut state = json!({"path": site.path, "source": site.quote});
    if let Some(function) = &site.function {
        state["function"] = json!(function);
    }
    state
}

fn build(
    file: &FileContext<'_>,
    pair: &Pair,
    hashes: &BTreeMap<PathBuf, String>,
    id: &str,
) -> (Value, Asked) {
    let mut questions = Questions::default();
    questions.ask(
        LOOK.into(),
        questions::copies_look(),
        id,
        SHARED_LOGIC,
        LOOK,
        Pass::First,
    );
    let sites: Vec<&Site> = sites(pair).take(SHOWN_SITES).collect();
    let state = json!({"sites": sites.iter().map(|s| site_state(s)).collect::<Vec<_>>()});
    let mut sources = vec![(file.path, file.source_hash)];
    for site in &sites {
        if site.path != file.path
            && !sources.iter().any(|(path, _)| *path == site.path.as_path())
            && let Some(hash) = hashes.get(&site.path)
        {
            sources.push((site.path.as_path(), hash.as_str()));
        }
    }
    request(file.model, "duplicate-pair", &sources, state, questions)
}
