//! Function simplification: packed function sources. Per function, a task Score
//! that can raise a review, a one-job Score that can clear, a flatten Noul and a
//! speculative task-kind Choice.
use super::{
    Asked, Detail, FileContext, FilePlan, Planned, Presence, Scope, UnitPlan, compact, identity,
    pack, questions, unique_ids,
};
use crate::{
    analysis::units::Unit, catalog::FUNCTION_SIMPLIFICATION, requests::TokenBudget, schema::Pass,
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
            judged.push((
                out.units.len() - 1,
                id,
                json!({"name": unit.name, "source": source}),
            ));
        }
    }
    for group in pack(judged, |(_, _, item)| item) {
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
                out.units[item.0].presence = Presence::NeedsContext;
                out.units[item.0].recheck = None;
            }
        }
    }
}

fn build(
    file: &FileContext<'_>,
    items: &[(usize, String, Value)],
    callees: Option<Vec<Value>>,
) -> (Value, Asked) {
    let pass = if callees.is_some() {
        Pass::Recheck
    } else {
        Pass::First
    };
    let mut questions = Map::new();
    let mut asked = Asked::default();
    for (index, (_, id, _)) in items.iter().enumerate() {
        let path = format!("functions[{index}].source");
        for (question, body) in [
            ("tasks", questions::function_tasks(&path, callees.is_some())),
            (
                "one_job",
                questions::function_one_job(&path, callees.is_some()),
            ),
            ("flatten", questions::function_flatten(&path)),
            ("task_kind", questions::function_task_kind(&path)),
        ] {
            asked.ask(
                &mut questions,
                format!("f{index}_{question}"),
                body,
                id,
                FUNCTION_SIMPLIFICATION,
                question,
                pass,
            );
        }
    }
    let mut state = json!({
        "file": file.file_state(),
        "functions": items.iter().map(|(_, _, item)| item.clone()).collect::<Vec<_>>(),
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
    let item = (
        0,
        id.to_string(),
        json!({"name": unit.name, "source": unit.source(file.source)}),
    );
    let (request, asked) = build(file, &[item], Some(callees));
    budget.fits(&request).then_some((request, asked))
}
