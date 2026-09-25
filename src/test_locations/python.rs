//! Python tests: `unittest` and pytest test classes, and top-level `test*`
//! functions in the files pytest collects.
use super::child_text;
use std::path::Path;
use tree_sitter::Node;

/// pytest collects top-level `test*` functions and `Test*` classes only from
/// `test_*.py` and `*_test.py`.
pub(crate) fn pytest_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
}

/// A `unittest` `TestCase` subclass, or a `Test*` class in a file pytest
/// collects: elsewhere a `TestClient` or `TestResponse` is library code.
pub(crate) fn python_test_class(node: Node<'_>, source: &str, pytest: bool) -> bool {
    node.kind() == "class_definition"
        && (pytest
            && node
                .child_by_field_name("name")
                .is_some_and(|name| child_text(name, source).starts_with("Test"))
            || node
                .child_by_field_name("superclasses")
                .is_some_and(|bases| child_text(bases, source).contains("TestCase")))
}

/// Python test classes anywhere, and top-level `test*` functions in pytest files.
pub(super) fn python_test_span(
    node: Node<'_>,
    source: &str,
    pytest: bool,
) -> Option<(usize, usize)> {
    let definition = if node.kind() == "decorated_definition" {
        node.child_by_field_name("definition")?
    } else if matches!(node.kind(), "class_definition" | "function_definition")
        && node.parent()?.kind() != "decorated_definition"
    {
        node
    } else {
        return None;
    };
    let test = python_test_class(definition, source, pytest)
        || pytest
            && definition.kind() == "function_definition"
            && node.parent()?.kind() == "module"
            && definition
                .child_by_field_name("name")
                .is_some_and(|name| child_text(name, source).starts_with("test"));
    test.then(|| (node.start_byte(), node.end_byte()))
}
