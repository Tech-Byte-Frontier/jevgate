//! Shared logic: one candidate pair per request, entity-alignment style.
use super::{
    Asked, Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan, identity,
    questions, request,
};
use crate::{
    analysis::{
        clones::{Candidates, Pair, Site},
        test_map::TestCase,
    },
    catalog::SHARED_LOGIC,
    schema::{Location, Pass},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) fn plan(
    file: &FileContext<'_>,
    candidates: &Candidates,
    cases: &BTreeMap<PathBuf, Vec<TestCase>>,
    test_lines: &[std::ops::Range<usize>],
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
        let (request, asked) = build(file, pair, hashes, &id, false);
        let presence = if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
            Presence::Judged
        } else {
            Presence::NeedsContext
        };
        let recheck = (presence == Presence::Judged
            && (pair.a.function_source.is_some() || pair.b.function_source.is_some()))
        .then(|| build(file, pair, hashes, &id, true))
        .filter(|(request, _)| file.budget.fits(request));
        let in_case = |site: &Site| {
            cases.get(&site.path).is_some_and(|cases| {
                cases
                    .iter()
                    .any(|case| case.line <= site.start_line && site.end_line <= case.end_line)
            })
        };
        let own_cases = cases.get(file.path).map_or(&[][..], Vec::as_slice);
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
            locations: [&pair.a, &pair.b]
                .into_iter()
                .chain(&pair.copies)
                .map(location)
                .collect(),
            quote: Some(pair.a.quote.clone()),
            lines: pair.a.end_line + 1 - pair.a.start_line,
            identity: identity(&[
                pair.a.function.as_deref().unwrap_or(""),
                &pair.b.path.to_string_lossy(),
                pair.b.function.as_deref().unwrap_or(""),
                &pair.normalized,
            ]),
            detail: Detail::Pair {
                differences: pair.differences.clone(),
                within_test: pair.b.path == file.path
                    && own_cases.iter().any(|case| {
                        [&pair.a, &pair.b].iter().all(|site| {
                            case.line <= site.start_line && site.end_line <= case.end_line
                        })
                    }),
                in_tests: test_lines.iter().any(|l| l.contains(&pair.a.start_line)),
                in_cases: [&pair.a, &pair.b]
                    .into_iter()
                    .chain(&pair.copies)
                    .all(in_case),
            },
            recheck: recheck.map(Into::into),
        });
    }
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

fn site_state(site: &Site, recheck: bool) -> Value {
    let mut state = json!({"path": site.path, "source": site.quote});
    if let Some(function) = &site.function {
        state["function"] = json!(function);
    }
    if recheck && let Some(source) = &site.function_source {
        state["function_source"] = json!(source);
    }
    state
}

fn build(
    file: &FileContext<'_>,
    pair: &Pair,
    hashes: &BTreeMap<PathBuf, String>,
    id: &str,
    recheck: bool,
) -> (Value, Asked) {
    let pass = if recheck { Pass::Recheck } else { Pass::First };
    let mut questions = Questions::default();
    for (question, body) in [
        ("same", questions::duplicate_same(recheck)),
        ("only_differences", questions::duplicate_only_differences()),
        ("required", questions::duplicate_required()),
    ] {
        questions.ask(question.into(), body, id, SHARED_LOGIC, question, pass);
    }
    let state = json!({
        "site_a": site_state(&pair.a, recheck),
        "site_b": site_state(&pair.b, recheck),
        "differences": pair.differences.iter().map(|d| json!({"site_a": d.a, "site_b": d.b})).collect::<Vec<_>>(),
    });
    let stage = if recheck { "recheck" } else { "duplicate-pair" };
    let mut sources = vec![(file.path, file.source_hash)];
    if pair.b.path != file.path
        && let Some(hash) = hashes.get(&pair.b.path)
    {
        sources.push((pair.b.path.as_path(), hash.as_str()));
    }
    request(file.model, stage, &sources, state, questions)
}
