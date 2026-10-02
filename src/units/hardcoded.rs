//! Hardcoded values: each function that holds literal values, asked one
//! look-here question in the function packs every rule shares (`packs`), and
//! one unit per file for its module-level constants. A coding agent verifies
//! each flag: it names the value or reads it from configuration or data, or
//! dismisses the finding with a reason. Questions on the kind of each value,
//! its locate Choice and a benign-kind recheck once decided these findings;
//! naming the kinds of values worth a look and letting the agent find the
//! one is simpler and flagged more of the values a reviewer wanted changed.
use super::{
    Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan, compact, identity,
    outcome::LOOK,
    packs::{Ask, FunctionAsk},
    questions, unique_ids,
};
use crate::{
    analysis::{literals::Constant, units::Unit},
    catalog::HARDCODED_VALUES,
    schema::Pass,
};
use serde_json::{Value, json};

/// Plan a unit per function that holds literal values, each paired with its
/// position among its file's parsed units, and one for the module constants;
/// return what the first pass asks about the functions.
pub(super) fn plan(
    file: &FileContext<'_>,
    units: &[(usize, &Unit)],
    constants: &[Constant],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) -> Vec<FunctionAsk> {
    let units: Vec<(usize, &Unit)> = units
        .iter()
        .copied()
        .filter(|(_, u)| !u.literals.is_empty())
        .collect();
    let ids = unique_ids("values", units.iter().map(|(_, u)| u.name.as_str()));
    let mut asks = Vec::new();
    for ((position, unit), id) in units.into_iter().zip(ids) {
        let source = unit.source(file.source);
        out.units.push(UnitPlan {
            rule: HARDCODED_VALUES,
            id,
            name: unit.name.clone(),
            presence: Presence::Judged,
            locations: vec![file.location(unit.line, unit.end_line, Some(&unit.name))],
            quote: None,
            lines: unit.lines(),
            identity: identity(&[&unit.name, &compact(source)]),
            detail: Detail::Values {
                values: unit.literals.iter().map(|l| l.text.clone()).collect(),
            },
            recheck: None,
        });
        asks.push(FunctionAsk {
            position,
            name: unit.name.clone(),
            source: source.to_string(),
            ask: Ask::Values {
                unit: out.units.len() - 1,
            },
        });
    }
    if !constants.is_empty() {
        plan_constants(file, constants, out, requests);
    }
    asks
}

/// The look-here question about the values of `functions[index]`, keyed
/// apart from function simplification's look-here question in the same pack.
pub(super) fn ask(questions: &mut Questions, index: usize, id: &str) {
    questions.ask(
        format!("f{index}_values"),
        questions::values_look(&format!("functions[{index}].source")),
        id,
        HARDCODED_VALUES,
        LOOK,
        Pass::First,
    );
}

const CONSTANTS_ID: &str = "constants";

/// One unit for the file's module-level constants, asked whether one fixes
/// a value worth a look: a name already explains each value, so only one
/// that differs between environments or singles out one record counts.
fn plan_constants(
    file: &FileContext<'_>,
    constants: &[Constant],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let mut questions = Questions::default();
    questions.ask(
        LOOK.into(),
        questions::constants_look(),
        CONSTANTS_ID,
        HARDCODED_VALUES,
        LOOK,
        Pass::First,
    );
    let listed: Vec<Value> = constants
        .iter()
        .map(|c| match &c.value {
            Some(value) => json!({"name": c.name, "value": value}),
            None => json!({"name": c.name, "values": c.values}),
        })
        .collect();
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
