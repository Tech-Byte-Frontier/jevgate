//! PHP syntax the shared analysis reads differently: the names a `use`
//! declaration imports, callees of its call expressions, callbacks
//! registered through calls (`$app->get('/users', function (…) {…})`,
//! `Route::get(…)`), the top-level statements of a page script, which run
//! on every request, and PHPUnit and Pest tests.
mod registration;
mod script;
mod test_cases;

pub(crate) use registration::*;
pub(crate) use script::*;
pub(crate) use test_cases::*;

use super::text;
use std::collections::BTreeSet;
use tree_sitter::Node;

/// Call expressions, whose callee is a `function` or `name` field.
pub(crate) const CALLS: &[&str] = &[
    "function_call_expression",
    "member_call_expression",
    "nullsafe_member_call_expression",
    "scoped_call_expression",
];

/// Constructs that write text into the page or run another file: sites
/// even without a call.
pub(crate) const OUTPUT: &[&str] = &[
    "echo_statement",
    "print_intrinsic",
    "include_expression",
    "include_once_expression",
    "require_expression",
    "require_once_expression",
];

/// Whether this file is PHP, by its extension.
pub(crate) fn file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "php" | "phtml"))
}

/// The node that names what a PHP call or `new` expression calls.
pub(crate) fn callee(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "function_call_expression" => node.child_by_field_name("function"),
        "member_call_expression" | "nullsafe_member_call_expression" | "scoped_call_expression" => {
            node.child_by_field_name("name")
        }
        "object_creation_expression" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|c| matches!(c.kind(), "name" | "qualified_name"))
        }
        _ => None,
    }
}

/// The last name segment of a PHP callee: `query` in `$db->query(…)`,
/// `Response` in `new \Slim\Psr7\Response()`.
pub(crate) fn callee_name(node: Node<'_>, source: &str) -> Option<String> {
    let callee = callee(node)?;
    let name = match callee.kind() {
        "name" => callee,
        "qualified_name" => {
            let mut cursor = callee.walk();
            callee
                .named_children(&mut cursor)
                .filter(|c| c.kind() == "name")
                .last()?
        }
        _ => return None,
    };
    Some(text(name, source).to_string())
}

/// The argument list of a PHP call or `new` expression.
pub(crate) fn arguments(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("arguments").or_else(|| {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|c| c.kind() == "arguments")
    })
}

/// The value an `argument` node passes, past a named argument's label.
pub(crate) fn argument_value(argument: Node<'_>) -> Option<Node<'_>> {
    if argument.kind() != "argument" {
        return Some(argument);
    }
    let mut cursor = argument.walk();
    argument.named_children(&mut cursor).last()
}

/// The names a `use` declaration brings into scope: each clause's alias or
/// the last segment of its name.
pub(crate) fn imports(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
    if node.kind() == "namespace_use_clause" {
        let name = node
            .child_by_field_name("alias")
            .or_else(|| {
                let mut cursor = node.walk();
                let last = node.named_children(&mut cursor).last()?;
                match last.kind() {
                    "qualified_name" => {
                        let mut inner = last.walk();
                        last.named_children(&mut inner)
                            .filter(|c| c.kind() == "name")
                            .last()
                    }
                    _ => Some(last),
                }
            })
            .map(|n| text(n, source).to_string());
        names.extend(name);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        imports(child, source, names);
    }
}
