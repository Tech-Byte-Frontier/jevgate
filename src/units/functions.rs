//! Function simplification: packed function sources. Per function, a Score on
//! whether splitting would help a reader and, only where the parser finds deep
//! nesting or a long branch chain, a Score on whether flattening would help.
use super::{
    Asked, Detail, FileContext, FilePlan, PACK_ITEMS, Planned, Presence, Scope, UnitPlan, compact,
    identity, pack, questions, unique_ids,
};
use crate::{
    analysis::units::Unit, catalog::FUNCTION_SIMPLIFICATION, schema::Pass,
    token_budget::TokenBudget,
};
use serde_json::{Map, Value, json};

const CALLEES: usize = 16;

pub(super) fn plan(
    file: &FileContext<'_>,
    units: &[&Unit],
    scope: &Scope<'_>,
    budget: &TokenBudget,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let ids = unique_ids("function", units.iter().map(|u| u.name.as_str()));
    let mut judged = Vec::new();
    for (unit, id) in units.iter().zip(ids) {
        let source = unit.source(file.source);
        let presence = if unit.too_small() {
            Presence::TooSmall
        } else {
            Presence::Judged
        };
        let recheck = (presence == Presence::Judged)
            .then(|| recheck(file, unit, &id, scope, budget))
            .flatten();
        out.units.push(UnitPlan {
            rule: FUNCTION_SIMPLIFICATION,
            id: id.clone(),
            name: unit.name.clone(),
            presence,
            locations: vec![file.location(unit.line, unit.end_line, Some(&unit.name))],
            quote: None,
            lines: unit.lines(),
            identity: identity(&[&unit.name, &compact(source)]),
            detail: Detail::Function,
            recheck,
        });
        if presence == Presence::Judged {
            judged.push(Item {
                index: out.units.len() - 1,
                id,
                nested: unit.deeply_nested(),
                state: json!({"name": unit.name, "source": source}),
            });
        }
    }
    for group in pack(judged, PACK_ITEMS, |item| &item.state) {
        let (request, asked) = build(file, &group, None);
        if budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
            continue;
        }
        // A pack that is too large is sent one function at a time.
        for item in group {
            let (request, asked) = build(file, std::slice::from_ref(&item), None);
            if budget.fits(&request) {
                requests.push(Planned {
                    owner: file.owner,
                    request,
                    asked,
                });
            } else {
                out.units[item.index].presence = Presence::NeedsContext;
                out.units[item.index].recheck = None;
            }
        }
    }
}

#[derive(Clone)]
struct Item {
    index: usize,
    id: String,
    /// Deep nesting or a long branch chain: flattening is also asked.
    nested: bool,
    state: Value,
}

fn build(file: &FileContext<'_>, items: &[Item], callees: Option<Vec<Value>>) -> (Value, Asked) {
    let pass = if callees.is_some() {
        Pass::Recheck
    } else {
        Pass::First
    };
    let mut questions = Map::new();
    let mut asked = Asked::default();
    for (index, item) in items.iter().enumerate() {
        let path = format!("functions[{index}].source");
        let mut asked_here = vec![("split", questions::function_split(&path, callees.is_some()))];
        if item.nested {
            asked_here.push(("flatten", questions::function_flatten(&path)));
        }
        for (question, body) in asked_here {
            asked.ask(
                &mut questions,
                format!("f{index}_{question}"),
                body,
                &item.id,
                FUNCTION_SIMPLIFICATION,
                question,
                pass,
            );
        }
    }
    let mut state = json!({
        "file": file.file_state(),
        "functions": items.iter().map(|item| item.state.clone()).collect::<Vec<_>>(),
    });
    if let Some(callees) = callees {
        state["callees"] = json!(callees);
    }
    let stage = if pass == Pass::Recheck {
        "recheck"
    } else {
        "functions"
    };
    (file.request(stage, state, questions), asked)
}

/// The same questions about one function, with the signatures it calls.
fn recheck(
    file: &FileContext<'_>,
    unit: &Unit,
    id: &str,
    scope: &Scope<'_>,
    budget: &TokenBudget,
) -> Option<(Value, Asked)> {
    let mut callees = Vec::new();
    for (_, callee) in scope.scope_units() {
        if callees.len() == CALLEES {
            break;
        }
        if unit.calls.contains(&callee.short_name)
            && callee.name != unit.name
            && !callees.iter().any(|c: &Value| c["name"] == callee.name)
        {
            callees.push(json!({"name": callee.name, "signature": callee.signature}));
        }
    }
    if callees.is_empty() {
        return None;
    }
    let item = Item {
        index: 0,
        id: id.to_string(),
        nested: unit.deeply_nested(),
        state: json!({"name": unit.name, "source": unit.source(file.source)}),
    };
    let (request, asked) = build(file, &[item], Some(callees));
    budget.fits(&request).then_some((request, asked))
}
