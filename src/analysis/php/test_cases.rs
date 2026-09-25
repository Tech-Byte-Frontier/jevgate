//! PHPUnit test classes and methods, and Pest test and suite calls.
use super::{argument_value, arguments};
use crate::analysis::text;
use tree_sitter::Node;

/// A PHPUnit test class: `class …Test extends TestCase` or any `…TestCase` base.
pub(crate) fn test_class(node: Node<'_>, source: &str) -> bool {
    node.kind() == "class_declaration"
        && node
            .named_children(&mut node.walk())
            .any(|c| c.kind() == "base_clause" && text(c, source).trim_end().ends_with("TestCase"))
}

/// A test method of a PHPUnit class: named `test…`, or marked with a
/// `@test` doc comment or a `#[Test]` attribute.
pub(crate) fn test_method(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "method_declaration" {
        return false;
    }
    let named = node
        .child_by_field_name("name")
        .is_some_and(|n| text(n, source).starts_with("test"));
    let attributed = node
        .named_children(&mut node.walk())
        .any(|c| c.kind() == "attribute_list" && text(c, source).contains("Test"));
    let documented = node
        .prev_named_sibling()
        .is_some_and(|c| crate::analysis::is_comment(c) && text(c, source).contains("@test"));
    named || attributed || documented
}

/// A Pest statement: `test('…', fn)` or `it('…', fn)`, a case, or
/// `describe('…', fn)`, a suite; also with chained calls such as `->skip()`.
pub(crate) struct Pest {
    pub title: String,
    pub suite: bool,
}

pub(crate) fn pest_statement(node: Node<'_>, source: &str) -> Option<Pest> {
    if node.kind() != "expression_statement" {
        return None;
    }
    let mut call = node.named_child(0)?;
    while call.kind() == "member_call_expression" {
        call = call.child_by_field_name("object")?;
    }
    let function = call
        .child_by_field_name("function")
        .filter(|_| call.kind() == "function_call_expression")?;
    let suite = match text(function, source) {
        "test" | "it" => false,
        "describe" => true,
        _ => return None,
    };
    let first = arguments(call)?.named_child(0).and_then(argument_value)?;
    matches!(first.kind(), "string" | "encapsed_string").then(|| Pest {
        title: text(first, source).trim_matches(['"', '\'']).to_string(),
        suite,
    })
}
