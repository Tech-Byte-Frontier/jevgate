//! Hardcoded values: packed function sources with the literal values each one
//! uses, and one unit per file for its module-level constants. Per function,
//! Scores on whether a value belongs in configuration or deserves a name, and
//! a Noul on whether the function special-cases one identity. A unit left
//! undecided is asked, alone, whether every value is of an acceptable kind.
use super::{
    Detail, FileContext, FilePlan, PACK_ITEMS, Planned, Presence, Questions, UnitPlan, compact,
    identity, pack, questions, unique_ids,
};
use crate::{
    analysis::{literals::Constant, units::Unit},
    catalog::HARDCODED_VALUES,
    schema::Pass,
};
use serde_json::{Value, json};

pub(super) fn plan(
    file: &FileContext<'_>,
    units: &[&Unit],
    constants: &[Constant],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let units: Vec<&Unit> = units
        .iter()
        .copied()
        .filter(|u| !u.literals.is_empty())
        .collect();
    let ids = unique_ids("values", units.iter().map(|u| u.name.as_str()));
    let mut items = Vec::new();
    for (unit, id) in units.iter().zip(ids) {
        let source = unit.source(file.source);
        let values: Vec<&str> = unit.literals.iter().map(|l| l.text.as_str()).collect();
        let state = json!({"name": unit.name, "source": source, "values": values});
        out.units.push(UnitPlan {
            rule: HARDCODED_VALUES,
            id: id.clone(),
            name: unit.name.clone(),
            presence: Presence::Judged,
            locations: vec![file.location(unit.line, unit.end_line, Some(&unit.name))],
            quote: None,
            lines: unit.lines(),
            identity: identity(&[&unit.name, &compact(source)]),
            detail: Detail::Values {
                values: unit.literals.iter().map(|l| l.text.clone()).collect(),
            },
            recheck: benign_request(file, &id, json!({"functions": [state.clone()]}), true),
        });
        items.push((out.units.len() - 1, id, state));
    }
    for group in pack(items, PACK_ITEMS, |(_, _, state)| state) {
        send_or_split(file, group, out, requests);
    }
    if !constants.is_empty() {
        plan_constants(file, constants, out, requests);
    }
}

/// A pack that is too large is sent one function at a time.
fn send_or_split(
    file: &FileContext<'_>,
    group: Vec<(usize, String, Value)>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (request, asked) = functions_request(file, &group);
    if file.budget.fits(&request) {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
        return;
    }
    for item in group {
        let (request, asked) = functions_request(file, std::slice::from_ref(&item));
        if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        } else {
            out.units[item.0].presence = Presence::NeedsContext;
            out.units[item.0].recheck = None;
        }
    }
}

fn functions_request(
    file: &FileContext<'_>,
    items: &[(usize, String, Value)],
) -> (Value, super::Asked) {
    let mut questions = Questions::default();
    for (index, (_, id, _)) in items.iter().enumerate() {
        let values = format!("functions[{index}].values");
        let code = format!("functions[{index}].source");
        for (question, body) in [
            (
                "environment",
                questions::hardcoded_environment(&values, &format!("`{code}`")),
            ),
            ("magic", questions::hardcoded_magic(&values, &code)),
            ("special", questions::hardcoded_special(&code)),
        ] {
            questions.ask(
                format!("f{index}_{question}"),
                body,
                id,
                HARDCODED_VALUES,
                question,
                Pass::First,
            );
        }
    }
    let state = json!({
        "file": file.file_state(),
        "functions": items.iter().map(|(_, _, state)| state.clone()).collect::<Vec<_>>(),
    });
    file.request("values", state, questions)
}

const CONSTANTS_ID: &str = "constants";

/// One unit for the file's module-level constants: only the environment
/// question, since a name already explains each value.
fn plan_constants(
    file: &FileContext<'_>,
    constants: &[Constant],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let mut questions = Questions::default();
    questions.ask(
        "environment".into(),
        questions::hardcoded_environment("constants", "the program"),
        CONSTANTS_ID,
        HARDCODED_VALUES,
        "environment",
        Pass::First,
    );
    let listed: Vec<Value> = constants
        .iter()
        .map(|c| match &c.value {
            Some(value) => json!({"name": c.name, "value": value}),
            None => json!({"name": c.name, "values": c.values}),
        })
        .collect();
    let recheck = benign_request(
        file,
        CONSTANTS_ID,
        json!({"constants": listed.clone()}),
        false,
    );
    let state = json!({"file": file.file_state(), "constants": listed});
    let (request, asked) = file.request("constants", state, questions);
    let fits = file.budget.fits(&request);
    let names: Vec<&str> = constants.iter().map(|c| c.name.as_str()).collect();
    out.units.push(UnitPlan {
        rule: HARDCODED_VALUES,
        id: CONSTANTS_ID.into(),
        name: "module constants".into(),
        presence: if fits {
            Presence::Judged
        } else {
            Presence::NeedsContext
        },
        locations: constants
            .iter()
            .map(|c| file.location(c.line, c.end_line, Some(&c.name)))
            .collect(),
        quote: None,
        lines: constants.iter().map(|c| c.end_line + 1 - c.line).sum(),
        identity: identity(&names),
        detail: Detail::Constants {
            values: constants.iter().flat_map(|c| c.values.clone()).collect(),
        },
        recheck: recheck.filter(|_| fits),
    });
    if fits {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
    }
}

/// The benign-kind checks for one unit, sent only when a question stayed
/// undecided: every question for a function, the environment for constants.
fn benign_request(
    file: &FileContext<'_>,
    id: &str,
    evidence: Value,
    function: bool,
) -> Option<(Value, super::Asked)> {
    let (values, code, asked): (&str, &str, &[&'static str]) = if function {
        (
            "functions[0].values",
            "functions[0].source",
            &["environment", "magic", "special"],
        )
    } else {
        ("constants", "the program", &["environment"])
    };
    let mut questions = Questions::default();
    for question in asked {
        questions.ask(
            question.to_string(),
            questions::hardcoded_benign(question, values, code),
            id,
            HARDCODED_VALUES,
            super::outcome::benign_key(question),
            Pass::Recheck,
        );
    }
    let mut state = evidence;
    state["file"] = file.file_state();
    let (request, asked) = file.request("recheck", state, questions);
    file.budget.fits(&request).then_some((request, asked))
}
