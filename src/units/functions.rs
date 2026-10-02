//! Function simplification: per function, a Score on whether splitting would
//! help a reader and, only where the parser finds deep nesting or a long
//! branch chain, a Score on whether flattening would help, whose reviews the
//! default gate measures, and the look-here question on whether it could be
//! simpler, asked in the function packs every rule shares (`packs`). A split
//! review is then located with one Choice among the body's blocks.
use super::{
    Asked, Block, Detail, FileContext, FilePlan, Presence, Questions, Scope, UnitPlan, compact,
    identity,
    packs::{Ask, FunctionAsk},
    questions, unique_ids,
};
use crate::{analysis::units::Unit, catalog::FUNCTION_SIMPLIFICATION, schema::Pass};
use serde_json::{Value, json};

const CALLEES: usize = 16;

/// Plan a unit per function, each paired with its position among its file's
/// parsed units, and return what the first pass asks about those judged.
pub(super) fn plan(
    file: &FileContext<'_>,
    units: &[(usize, &Unit)],
    scope: &Scope<'_>,
    out: &mut FilePlan,
) -> Vec<FunctionAsk> {
    let ids = unique_ids("function", units.iter().map(|(_, u)| u.name.as_str()));
    let mut asks = Vec::new();
    for (&(position, unit), id) in units.iter().zip(ids) {
        let source = unit.source(file.source);
        let presence = if unit.too_small() {
            Presence::TooSmall
        } else {
            Presence::Judged
        };
        let recheck = (presence == Presence::Judged)
            .then(|| recheck(file, unit, &id, scope))
            .flatten();
        let blocks = blocks(file, unit);
        let locate = (presence == Presence::Judged && !blocks.is_empty())
            .then(|| locate(file, unit, &id, &blocks))
            .and_then(|built| file.fitting(built));
        out.units.push(UnitPlan {
            rule: FUNCTION_SIMPLIFICATION,
            id,
            name: unit.name.clone(),
            presence,
            locations: vec![file.location(unit.line, unit.end_line, Some(&unit.name))],
            quote: None,
            lines: unit.lines(),
            identity: identity(&[&unit.name, &compact(source)]),
            detail: Detail::Function {
                blocks,
                locate: locate.map(Into::into),
            },
            recheck: recheck.map(Into::into),
        });
        if presence == Presence::Judged {
            asks.push(FunctionAsk {
                position,
                name: unit.name.clone(),
                source: source.to_string(),
                ask: Ask::Split {
                    unit: out.units.len() - 1,
                    nested: unit.deeply_nested(),
                },
            });
        }
    }
    asks
}

/// The split Score about `functions[index]` and, where its nesting is deep
/// (`nested`), the flatten Score, whose reviews the default gate measures;
/// in the first pass, the look-here question, which flags the rest for a
/// coding agent to verify. A recheck's split question points at the callee
/// signatures sent beside it.
pub(super) fn ask(questions: &mut Questions, index: usize, id: &str, nested: bool, pass: Pass) {
    let path = format!("functions[{index}].source");
    let mut asked = vec![(
        "split",
        questions::function_split(&path, pass == Pass::Recheck),
    )];
    if nested {
        asked.push(("flatten", questions::function_flatten(&path)));
    }
    if pass == Pass::First {
        asked.push((super::outcome::LOOK, questions::function_look(&path)));
    }
    for (question, body) in asked {
        questions.ask(
            format!("f{index}_{question}"),
            body,
            id,
            FUNCTION_SIMPLIFICATION,
            question,
            pass,
        );
    }
}

/// The same questions about one function, with the signatures it calls.
fn recheck(
    file: &FileContext<'_>,
    unit: &Unit,
    id: &str,
    scope: &Scope<'_>,
) -> Option<(Value, Asked)> {
    // Callees of one family: a Kotlin function's `map` is no Python `map`,
    // and the other languages keep reading each other as before. The family
    // is looked up only for a name the unit calls: the pairs of functions
    // grow with the square of a scope's size.
    let family = crate::analysis::generic::family(file.path);
    let mut callees = Vec::new();
    for (path, _, callee) in scope.scope_units() {
        if callees.len() == CALLEES {
            break;
        }
        if unit.calls.contains(&callee.short_name)
            && callee.name != unit.name
            && crate::analysis::generic::family(path) == family
            && !callees.iter().any(|c: &Value| c["name"] == callee.name)
        {
            callees.push(json!({"name": callee.name, "signature": callee.signature}));
        }
    }
    if callees.is_empty() {
        return None;
    }
    let mut questions = Questions::default();
    ask(&mut questions, 0, id, unit.deeply_nested(), Pass::Recheck);
    let state = json!({
        "file": file.plain_state(),
        "functions": [{"name": unit.name, "source": unit.source(file.source)}],
        "callees": callees,
    });
    file.fitting(file.request("recheck", state, questions))
}

fn blocks(file: &FileContext<'_>, unit: &Unit) -> Vec<Block> {
    unit.blocks
        .iter()
        .enumerate()
        .map(|(i, range)| Block {
            id: format!("B{}", i + 1),
            location: file.location(
                crate::analysis::line_of(file.source, range.start),
                crate::analysis::line_of(file.source, range.end.saturating_sub(1)),
                Some(&unit.name),
            ),
        })
        .collect()
}

/// Which block of one function to extract: its signature and its body as blocks.
fn locate(file: &FileContext<'_>, unit: &Unit, id: &str, blocks: &[Block]) -> (Value, Asked) {
    let ids: Vec<String> = blocks.iter().map(|b| b.id.clone()).collect();
    let mut questions = Questions::default();
    questions.ask(
        "block".into(),
        questions::function_block(&ids),
        id,
        FUNCTION_SIMPLIFICATION,
        "block",
        Pass::Locate,
    );
    let state = json!({
        "file": file.plain_state(),
        "function": {
            "name": unit.name,
            "signature": unit.signature,
            "blocks": ids.iter().zip(&unit.blocks).map(|(id, range)| {
                json!({"id": id, "source": &file.source[range.clone()]})
            }).collect::<Vec<_>>(),
        },
    });
    file.request("locate", state, questions)
}
