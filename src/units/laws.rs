//! Bend 2 laws: one unit per claim that quantifies over its inputs and has
//! a comment directly above it, outside `PROOF.bend`. The law is the part of
//! a specification the compiler checks and its comment the part a person
//! reads, so Jev is asked whether the comment claims more than the law
//! states: a promise the law leaves out is one a definition can break while
//! every proof still passes. The law is shown as written and read in words
//! (`bend::Statement::reading`), with the defs it names by their
//! signatures, documentation and short bodies.
//!
//! A law without `for` or `exs` checks fixed values, as a unit test does,
//! and its comment says what the check samples; a lemma of a `PROOF.bend`
//! is a step of a proof, whose comment says how the proof goes. On the
//! Bend repository, the comments of both read as promising more than their
//! laws in most answers, where none did.
use super::{
    Asked, Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan, compact,
    identity, pack_runs, questions, unique_ids,
};
use crate::{
    analysis::units::{Kind, Unit},
    catalog::LAWS,
    schema::Pass,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Defs a law names that are shown with it, at most.
const DEFS: usize = 6;
/// A named def up to this many lines is shown whole; a longer one by its
/// signature and documentation.
const DEF_LINES: usize = 24;

/// A def the laws of a file can name, with the source of its file.
pub(super) struct Named<'a> {
    pub unit: &'a Unit,
    pub source: &'a str,
}

/// The claims of a file that have a comment, with the defs they name found
/// among `defs`: the file's own and those of the files it imports.
pub(super) fn plan(
    file: &FileContext<'_>,
    units: &[Unit],
    propositions: &BTreeSet<String>,
    defs: &[Named<'_>],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    out.rules.insert(LAWS, 0);
    if file.path.file_name().is_some_and(|n| n == "PROOF.bend") {
        return;
    }
    let claims: Vec<(&Unit, String)> = units
        .iter()
        .filter(|u| u.kind == Kind::Law)
        .filter(|u| {
            u.statement
                .as_ref()
                .is_some_and(|s| s.claim(propositions) && s.general())
        })
        .filter_map(|u| Some((u, comment(u, file.source)?)))
        .collect();
    let ids = unique_ids("law", claims.iter().map(|(u, _)| u.name.as_str()));
    let mut items = Vec::new();
    for ((unit, comment), id) in claims.into_iter().zip(ids) {
        let law = &file.source[declaration_start(unit, file.source)..unit.span.end];
        let named = named_defs(unit, defs);
        out.units.push(UnitPlan {
            rule: LAWS,
            id: id.clone(),
            name: unit.name.clone(),
            presence: Presence::Judged,
            locations: vec![file.location(unit.line, unit.end_line, Some(&unit.name))],
            quote: Some(comment.clone()),
            lines: unit.lines(),
            identity: identity(&[&unit.name, &compact(law), &compact(&comment)]),
            detail: Detail::Law,
            recheck: None,
        });
        items.push(Item {
            index: out.units.len() - 1,
            id,
            state: json!({
                "name": unit.name,
                "source": law,
                "reading": unit
                    .statement
                    .as_ref()
                    .map(|s| s.reading(propositions))
                    .unwrap_or_default(),
                "comment": comment,
                "defs": named,
            }),
        });
    }
    for group in pack_runs(
        items,
        |item| item.state["name"].as_str().unwrap_or_default(),
        |item| &item.state,
    ) {
        let (request, asked) = build(file, &group);
        if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
            continue;
        }
        for item in group {
            let (request, asked) = build(file, std::slice::from_ref(&item));
            if file.budget.fits(&request) {
                requests.push(Planned {
                    owner: file.owner,
                    request,
                    asked,
                });
            } else {
                out.units[item.index].presence = Presence::NeedsContext;
            }
        }
    }
}

struct Item {
    index: usize,
    id: String,
    state: Value,
}

fn build(file: &FileContext<'_>, items: &[Item]) -> (Value, Asked) {
    let mut questions = Questions::default();
    for (index, item) in items.iter().enumerate() {
        questions.ask(
            format!("l{index}_states"),
            questions::law_states(index),
            &item.id,
            LAWS,
            "states",
            Pass::First,
        );
    }
    let mut state = json!({
        "file": file.plain_state(),
        "laws": items.iter().map(|item| item.state.clone()).collect::<Vec<_>>(),
    });
    if let Some(header) = header(file.source) {
        state["file"]["comment"] = json!(header);
    }
    file.request("laws", state, questions)
}

/// The comment that opens the file, before its first line of code: what a
/// `LAWS.bend` says its laws pin, such as a server's pure part, or the task
/// an eval's laws state.
fn header(source: &str) -> Option<String> {
    let lines: Vec<&str> = source
        .lines()
        .map(str::trim)
        .take_while(|line| line.is_empty() || line.starts_with('#') && !line.starts_with("#|"))
        .filter(|line| line.starts_with('#'))
        .collect();
    let words: usize = lines
        .iter()
        .map(|line| line.trim_start_matches('#').split_whitespace().count())
        .sum();
    (words >= MIN_WORDS).then(|| lines.join("\n"))
}

/// Words a comment's prose needs to describe a law: `# solution` and a
/// section's title above it do not.
const MIN_WORDS: usize = 3;

/// The comment lines directly above a law, without the section headings
/// among them (a title over a rule of dashes, as Base heads `# Equal` over
/// `# -----`); none when too little prose remains.
fn comment(unit: &Unit, source: &str) -> Option<String> {
    let above = &source[unit.span.start..declaration_start(unit, source)];
    let lines: Vec<&str> = above
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('#'))
        .collect();
    let rule = |line: &str| {
        let text = line.trim_start_matches('#').trim();
        !text.is_empty() && text.chars().all(|c| matches!(c, '-' | '=' | '#' | '*'))
    };
    let kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, line)| !rule(line) && !lines.get(i + 1).is_some_and(|next| rule(next)))
        .map(|(_, line)| *line)
        .collect();
    let words: usize = kept
        .iter()
        .map(|line| line.trim_start_matches('#').split_whitespace().count())
        .sum();
    (words >= MIN_WORDS).then(|| kept.join("\n"))
}

/// The byte where the law's own line starts, after the comments above it.
fn declaration_start(unit: &Unit, source: &str) -> usize {
    source
        .match_indices('\n')
        .nth(unit.line.saturating_sub(2))
        .filter(|_| unit.line > 1)
        .map_or(0, |(at, _)| at + 1)
        .max(unit.span.start)
}

/// The defs a law's statement and clauses call, by their full or unaliased
/// names (`Srv.http_response` names `http_response` of the file imported as
/// `Srv`), in the order found.
fn named_defs(law: &Unit, defs: &[Named<'_>]) -> Vec<Value> {
    let mut shown = Vec::new();
    for call in &law.calls {
        if shown.len() == DEFS {
            break;
        }
        let Some(named) = defs.iter().find(|d| d.unit.name == *call) else {
            continue;
        };
        let unit = named.unit;
        let mut def = json!({"name": unit.name, "signature": unit.signature});
        if !unit.doc.is_empty() {
            def["doc"] = json!(unit.doc);
        }
        if unit.lines() <= DEF_LINES {
            def["source"] = json!(unit.source(named.source));
        }
        if !shown.iter().any(|d: &Value| d["name"] == def["name"]) {
            shown.push(def);
        }
    }
    shown
}
