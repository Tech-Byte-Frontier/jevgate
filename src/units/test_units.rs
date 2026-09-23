//! Test quality: one request per test for value checks and one per candidate
//! redundant pair.
use super::{
    Asked, Detail, FileContext, FilePlan, Planned, Presence, TEST_PACK_ITEMS, UnitPlan, compact,
    identity, pack, questions, unique_ids,
};
use crate::{
    analysis::test_map::{self, TestCase},
    catalog::{TEST_REDUNDANCY, TEST_VALUE},
    requests::TokenBudget,
    schema::Pass,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const SUBJECTS: usize = 16;

fn subject_state(names: &[&String], subjects: &BTreeMap<String, String>) -> Vec<Value> {
    let mut seen = Vec::new();
    for name in names {
        if seen.len() < SUBJECTS && !seen.contains(name) {
            seen.push(*name);
        }
    }
    seen.iter()
        .map(|name| json!({"name": name, "signature": subjects.get(*name).cloned().unwrap_or_default()}))
        .collect()
}

pub(super) fn plan_values(
    file: &FileContext<'_>,
    cases: &[TestCase],
    subjects: &BTreeMap<String, String>,
    budget: &TokenBudget,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let ids = unique_ids("test", cases.iter().map(|c| c.name.as_str()));
    let mut items = Vec::new();
    for (case, id) in cases.iter().zip(ids) {
        let source = case.source(file.source);
        out.units.push(UnitPlan {
            rule: TEST_VALUE,
            id: id.clone(),
            name: case.name.clone(),
            presence: Presence::Judged,
            locations: vec![file.location(case.line, case.end_line, Some(&case.name))],
            quote: None,
            lines: case.end_line + 1 - case.line,
            identity: identity(&[&case.name, &compact(source)]),
            detail: Detail::Test,
            recheck: None,
        });
        items.push((
            out.units.len() - 1,
            id,
            case,
            json!({"name": case.name, "source": source}),
        ));
    }
    for group in pack(items, TEST_PACK_ITEMS, |(_, _, _, item)| item) {
        let mut questions = Map::new();
        let mut asked = Asked::default();
        for (index, (_, id, _, _)) in group.iter().enumerate() {
            let path = format!("tests[{index}].source");
            for (question, body) in [
                ("internal", questions::test_internal(&path)),
                ("own_logic", questions::test_own_logic(&path)),
                ("mock_only", questions::test_mock_only(&path)),
                ("several", questions::test_several(&path)),
            ] {
                asked.ask(
                    &mut questions,
                    format!("t{index}_{question}"),
                    body,
                    id,
                    TEST_VALUE,
                    question,
                    Pass::First,
                );
            }
        }
        let names: Vec<&String> = group
            .iter()
            .flat_map(|(_, _, case, _)| case.subjects.iter())
            .collect();
        let state = json!({
            "file": file.file_state(),
            "tests": group.iter().map(|(_, _, _, item)| item.clone()).collect::<Vec<_>>(),
            "subjects": subject_state(&names, subjects),
        });
        let request = file.request("tests", state, questions);
        if budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        } else {
            for (unit, ..) in group {
                out.units[unit].presence = Presence::NeedsContext;
            }
        }
    }
}

pub(super) fn plan_pairs(
    file: &FileContext<'_>,
    cases: &[TestCase],
    subjects: &BTreeMap<String, String>,
    budget: &TokenBudget,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (pairs, omitted) = test_map::pairs(cases);
    out.rules.insert(TEST_REDUNDANCY, omitted);
    for pair in pairs {
        let (a, b) = (&cases[pair.a], &cases[pair.b]);
        let id = format!("test-pair:{}|{}", a.name, b.name);
        let mut questions = Map::new();
        let mut asked = Asked::default();
        for (question, body) in [
            ("overlap", questions::test_pair_overlap()),
            ("same_input", questions::test_pair_same_input()),
            ("same_outcome", questions::test_pair_same_outcome()),
        ] {
            asked.ask(
                &mut questions,
                question.into(),
                body,
                &id,
                TEST_REDUNDANCY,
                question,
                Pass::First,
            );
        }
        let state = json!({
            "test_a": {"name": a.name, "source": a.source(file.source)},
            "test_b": {"name": b.name, "source": b.source(file.source)},
            "subject": subject_state(&[&pair.subject], subjects).remove(0),
        });
        let request = file.request("test-pair", state, questions);
        let fits = budget.fits(&request);
        out.units.push(UnitPlan {
            rule: TEST_REDUNDANCY,
            id,
            name: format!("`{}` and `{}`", a.name, b.name),
            presence: if fits {
                Presence::Judged
            } else {
                Presence::NeedsContext
            },
            locations: vec![
                file.location(a.line, a.end_line, Some(&a.name)),
                file.location(b.line, b.end_line, Some(&b.name)),
            ],
            quote: None,
            lines: a.end_line + 1 - a.line + b.end_line + 1 - b.line,
            identity: identity(&[
                &a.name,
                &b.name,
                &compact(a.source(file.source)),
                &compact(b.source(file.source)),
            ]),
            detail: Detail::TestPair {
                names: [a.name.clone(), b.name.clone()],
                subject: pair.subject.clone(),
            },
            recheck: None,
        });
        if fits {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        }
    }
}
