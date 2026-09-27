//! Bend 2 laws: one unit per claim that quantifies over its inputs and has
//! a comment directly above it, outside files of proofs. The law is the part of
//! a specification the compiler checks and its comment the part a person
//! reads, so Jev is asked whether the comment claims more than the law
//! states: a promise the law leaves out is one a definition can break while
//! every proof still passes. The law is shown as written and read in words
//! (`bend::Statement::reading`), with the defs it names by their
//! signatures, documentation and short bodies.
//!
//! A law without `for` or `exs` checks fixed values, as a unit test does,
//! and its comment says what the check samples; a lemma of a file of
//! proofs (`bend::proof_file`) is a step of a proof, whose comment says how
//! the proof goes. On the Bend repository, the comments of both read as
//! promising more than their laws in most answers, where none did, and 14
//! of the 15 law findings in bend-collections' `proofs/` were wrong.
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
    if crate::analysis::bend::proof_file(file.path) {
        return;
    }
    let claims: Vec<(usize, &Unit, String)> = units
        .iter()
        .enumerate()
        .filter(|(_, u)| u.kind == Kind::Law)
        .filter(|(_, u)| {
            u.statement
                .as_ref()
                .is_some_and(|s| s.claim(propositions) && s.general())
        })
        .filter_map(|(at, u)| Some((at, u, comment(u, file.source)?)))
        .collect();
    let ids = unique_ids("law", claims.iter().map(|(_, u, _)| u.name.as_str()));
    let mut items = Vec::new();
    for ((at, unit, comment), id) in claims.into_iter().zip(ids) {
        let group = group(units, at, file.source);
        let law = group
            .iter()
            .map(|u| &file.source[declaration_start(u, file.source)..u.span.end])
            .collect::<Vec<_>>()
            .join("\n\n");
        let reading = |u: &Unit| {
            u.statement
                .as_ref()
                .map(|s| s.reading(propositions))
                .unwrap_or_default()
        };
        let reading = match group.as_slice() {
            [only] => reading(only),
            laws => laws
                .iter()
                .map(|u| format!("`{}`: {}", u.name, reading(u)))
                .collect::<Vec<_>>()
                .join(" "),
        };
        let named = named_defs(&group, defs);
        let state = json!({
            "name": unit.name,
            "source": law,
            "reading": reading,
            "comment": comment,
            "defs": named,
        });
        let recheck =
            Some(recheck(file, &id, &state)).filter(|(request, _)| file.budget.fits(request));
        out.units.push(UnitPlan {
            rule: LAWS,
            id: id.clone(),
            name: unit.name.clone(),
            presence: Presence::Judged,
            locations: vec![file.location(unit.line, unit.end_line, Some(&unit.name))],
            quote: Some(comment.clone()),
            lines: unit.lines(),
            identity: identity(&[&unit.name, &compact(&law), &compact(&comment)]),
            detail: Detail::Law,
            recheck: recheck.map(Into::into),
        });
        items.push(Item {
            index: out.units.len() - 1,
            id,
            state,
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

/// The Choice a law whose first answer stays undecided is asked: what its
/// comment says beyond the law.
fn recheck(file: &FileContext<'_>, id: &str, law: &Value) -> (Value, Asked) {
    let mut questions = Questions::default();
    questions.ask(
        "relation".into(),
        questions::law_relation(),
        id,
        LAWS,
        "relation",
        Pass::Recheck,
    );
    questions.ask(
        "fixed".into(),
        questions::law_fixed(),
        id,
        LAWS,
        "fixed",
        Pass::Recheck,
    );
    let mut state = json!({"file": file.plain_state(), "law": law});
    if let Some(header) = header(file.source) {
        state["file"]["comment"] = json!(header);
    }
    file.request("recheck", state, questions)
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
    prose(&lines)
}

/// Words a comment's prose needs to describe a law: `# solution` and a
/// section's title above it do not.
const MIN_WORDS: usize = 3;

/// Comment lines joined, when they hold enough words to describe a law.
fn prose(lines: &[&str]) -> Option<String> {
    let words: usize = lines
        .iter()
        .map(|line| line.trim_start_matches('#').split_whitespace().count())
        .sum();
    (words >= MIN_WORDS).then(|| lines.join("\n"))
}

/// The comment block directly above a law, without the section headings
/// among them (a title over a rule of dashes, as Base heads `# Equal` over
/// `# -----`); none when too little prose remains. A blank line ends the
/// block: the paragraph that opens a section above it speaks of the
/// section's laws together, and bulkhead's, which draws a restart claim
/// from several laws, read as a promise of the one below it.
fn comment(unit: &Unit, source: &str) -> Option<String> {
    let above = &source[unit.span.start..declaration_start(unit, source)];
    prose(&without_headings(&last_block(above)))
}

/// The comment lines of the last block of `above`, the lines after its last
/// blank line.
fn last_block(above: &str) -> Vec<&str> {
    let lines: Vec<&str> = above.lines().map(str::trim).collect();
    let end = lines
        .iter()
        .rposition(|line| !line.is_empty())
        .map_or(0, |i| i + 1);
    let start = lines[..end]
        .iter()
        .rposition(|line| line.is_empty())
        .map_or(0, |i| i + 1);
    lines[start..end]
        .iter()
        .copied()
        .filter(|line| line.starts_with('#'))
        .collect()
}

/// Comment lines without the rules of dashes and the titles over them, and
/// without the empty lines around what is left.
fn without_headings<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let text = |line: &str| line.trim_start_matches('#').trim().to_string();
    let rule = |line: &str| {
        let text = text(line);
        !text.is_empty() && text.chars().all(|c| matches!(c, '-' | '=' | '#' | '*'))
    };
    let mut kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, line)| !rule(line) && !lines.get(i + 1).is_some_and(|next| rule(next)))
        .map(|(_, line)| *line)
        .skip_while(|line| text(line).is_empty())
        .collect();
    while kept.last().is_some_and(|line| text(line).is_empty()) {
        kept.pop();
    }
    kept
}

/// The law at `at` and the laws right after it that have no comment of
/// their own, which its comment describes too: a comment saying an NFA is
/// "sound and complete" heads `nfa_sound` and the uncommented `nfa_complete`
/// below it.
fn group<'a>(units: &'a [Unit], at: usize, source: &str) -> Vec<&'a Unit> {
    let mut laws = vec![&units[at]];
    laws.extend(
        units[at + 1..]
            .iter()
            .take_while(|u| u.kind == Kind::Law && comment(u, source).is_none()),
    );
    laws
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

/// The defs the laws' statements and clauses call, by their full or
/// unaliased names (`Srv.http_response` names `http_response` of the file
/// imported as `Srv`), in name order.
fn named_defs(laws: &[&Unit], defs: &[Named<'_>]) -> Vec<Value> {
    let mut shown = Vec::new();
    let calls: std::collections::BTreeSet<&String> = laws.iter().flat_map(|l| &l.calls).collect();
    for call in calls {
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
