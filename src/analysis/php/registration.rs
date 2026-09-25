//! Closures a PHP file registers through calls, binds to a variable or
//! returns to the file that includes it.
use super::{CALLS, argument_value, arguments, callee_name};
use crate::analysis::text;
use tree_sitter::Node;

fn closure(node: Node<'_>) -> bool {
    matches!(node.kind(), "anonymous_function" | "arrow_function")
}

/// Whether an expression statement's expression is PHP syntax that can
/// register or bind a closure: a call, or an assignment to a `$variable`.
pub(crate) fn registers(expression: Node<'_>) -> bool {
    CALLS.contains(&expression.kind())
        || expression.kind() == "assignment_expression"
            && expression
                .child_by_field_name("left")
                .is_some_and(|l| l.kind() == "variable_name")
}

/// Closures a top-level statement registers through a call, named by the
/// registration: `$app->get('/users/{id}')` or `Route::post('/pages')`.
/// Each call of a chain (`$app->get(…)->add(…)`) registers its last closure
/// argument. A closure assigned to a variable (`$handler = function …`) is
/// named by the variable.
pub(crate) fn registered_callbacks<'t>(
    statement: Node<'t>,
    source: &str,
) -> Vec<(String, Node<'t>)> {
    let Some(expression) = statement.named_child(0) else {
        return Vec::new();
    };
    if expression.kind() == "assignment_expression" {
        return expression
            .child_by_field_name("right")
            .filter(|r| closure(*r))
            .zip(expression.child_by_field_name("left"))
            .map(|(right, left)| vec![(text(left, source).to_string(), right)])
            .unwrap_or_default();
    }
    let mut found = Vec::new();
    let mut call = Some(expression).filter(|e| CALLS.contains(&e.kind()));
    while let Some(current) = call {
        let values: Vec<Node<'t>> = arguments(current)
            .map(|a| {
                let mut cursor = a.walk();
                a.named_children(&mut cursor)
                    .filter_map(argument_value)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(handler) = values.iter().rev().find(|n| closure(**n)) {
            found.push((registration(current, &values, source), *handler));
        }
        call = current
            .child_by_field_name("object")
            .filter(|o| CALLS.contains(&o.kind()));
    }
    found.reverse();
    found
}

/// The name of the closure a call registers: the chain's receiver, the
/// method and the first string argument, as `$app->get('/users/{id}')` or
/// `Route::post('/pages')`.
fn registration(call: Node<'_>, values: &[Node<'_>], source: &str) -> String {
    let path = values
        .first()
        .filter(|a| matches!(a.kind(), "string" | "encapsed_string"))
        .map_or("…", |a| text(*a, source));
    let receiver = call
        .child_by_field_name("object")
        .or_else(|| call.child_by_field_name("scope"))
        .map(|o| root(o, source));
    let method = callee_name(call, source).unwrap_or_default();
    match (receiver, call.kind()) {
        (Some(receiver), "scoped_call_expression") => format!("{receiver}::{method}({path})"),
        (Some(receiver), _) => format!("{receiver}->{method}({path})"),
        (None, _) => format!("{method}({path})"),
    }
}

/// The leftmost receiver of a chain such as `$app->group(…)->add(…)`.
fn root(node: Node<'_>, source: &str) -> String {
    let mut node = node;
    while let Some(inner) = node
        .child_by_field_name("object")
        .or_else(|| node.child_by_field_name("scope"))
    {
        node = inner;
    }
    text(node, source).to_string()
}

/// The closure a top-level `return function (…) {…};` hands to the file
/// that includes it, as configuration files of Slim and Laravel do.
pub(crate) fn returned_closure(statement: Node<'_>) -> Option<Node<'_>> {
    (statement.kind() == "return_statement"
        && statement.parent().is_some_and(|p| p.kind() == "program"))
    .then(|| statement.named_child(0))
    .flatten()
    .filter(|n| closure(*n))
}

/// The name of the unit a returned closure becomes.
pub(crate) const RETURNED_CLOSURE: &str = "returned closure";
