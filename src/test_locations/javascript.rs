//! JavaScript and TypeScript tests: `describe`, `it` and `test` calls and
//! their hooks, as statements.
use super::child_text;
use tree_sitter::Node;

pub(super) fn javascript_test_span(node: Node<'_>, source: &str) -> Option<(usize, usize)> {
    if node.kind() != "call_expression" || !is_test_call(&callee(node, source)) {
        return None;
    }
    let statement = statement_span(node)?;
    Some((statement.start_byte(), statement.end_byte()))
}

fn is_test_call(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "describe",
        "xdescribe",
        "test",
        "xtest",
        "fdescribe",
        "it",
        "xit",
        "fit",
        "beforeEach",
        "afterEach",
        "beforeAll",
        "afterAll",
    ];
    NAMES
        .iter()
        .any(|candidate| name == *candidate || name.starts_with(&format!("{candidate}.")))
}

fn callee(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("function")
        .map(|child| child_text(child, source).trim().to_string())
        .unwrap_or_default()
}

/// The statement a test call makes up: the call itself, awaited or at the
/// head of a call chain such as `test.each(rows)("name", fn)`. A call inside
/// another expression, such as `test && test(value)` in library code, is not
/// a test declaration.
fn statement_span(node: Node<'_>) -> Option<Node<'_>> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "expression_statement" => return Some(parent),
            "call_expression" | "member_expression" | "await_expression" => current = parent,
            _ => return None,
        }
    }
    None
}
