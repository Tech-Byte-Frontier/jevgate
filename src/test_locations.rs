//! Structural tests a parser can locate: Rust test attributes and `cfg(test)`
//! modules, JavaScript and TypeScript `describe`/`it`/`test` calls, Python
//! test classes and pytest functions, Go `Test…(t *testing.T)` functions, C#
//! classes of xUnit, NUnit or MSTest tests, and Ruby RSpec groups and examples
//! and Minitest or Rails test classes. Syntax locates tests; it never judges
//! them.
use crate::{analysis::ruby, schema::SourceRange};
use anyhow::Result;
use std::path::Path;
use tree_sitter::Node;

/// Test code a parser can locate in one file.
pub(crate) struct Located {
    pub ranges: Vec<SourceRange>,
    /// Why some test syntax could not be separated.
    pub unresolved: Vec<String>,
    /// The whole file compiles only for tests (`#![cfg(test)]`).
    pub whole_file: bool,
}

pub(crate) fn locate_tests(path: &Path, source: &str) -> Result<Located> {
    let Some(tree) = crate::syntax::parse(path, source)? else {
        return Ok(Located {
            ranges: Vec::new(),
            unresolved: Vec::new(),
            whole_file: false,
        });
    };
    let root = tree.root_node();
    if whole_file_cfg_test(root, source) {
        return Ok(Located {
            ranges: Vec::new(),
            unresolved: Vec::new(),
            whole_file: true,
        });
    }
    let mut spans = Vec::new();
    walk(root, source, pytest_file(path), &mut spans);
    let mut ranges = Vec::new();
    let mut unresolved = Vec::new();
    for (start, end) in merge_spans(spans) {
        if owns_lines(source, start, end) {
            ranges.push(line_range(source, start, end));
        } else {
            unresolved.push("test syntax shares a line with other code".into());
        }
    }
    Ok(Located {
        ranges,
        unresolved,
        whole_file: false,
    })
}

fn walk(node: Node<'_>, source: &str, pytest: bool, spans: &mut Vec<(usize, usize)>) {
    if let Some(span) = rust_test_span(node, source)
        .or_else(|| javascript_test_span(node, source))
        .or_else(|| python_test_span(node, source, pytest))
        .or_else(|| go_test_function(node, source).then(|| (node.start_byte(), node.end_byte())))
        .or_else(|| csharp_test_span(node, source))
        .or_else(|| {
            (ruby_test_call(node, source) || ruby_test_class(node, source))
                .then(|| (node.start_byte(), node.end_byte()))
        })
    {
        spans.push(span);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, source, pytest, spans);
    }
}

/// A Go test, benchmark or fuzz function: `func TestX(t *testing.T)`.
pub(crate) fn go_test_function(node: Node<'_>, source: &str) -> bool {
    node.kind() == "function_declaration"
        && node.child_by_field_name("name").is_some_and(|name| {
            let name = child_text(name, source);
            ["Test", "Benchmark", "Fuzz"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
        && node.child_by_field_name("parameters").is_some_and(|p| {
            let parameters = child_text(p, source);
            ["*testing.T", "*testing.B", "*testing.F"]
                .iter()
                .any(|kind| parameters.contains(kind))
        })
}

/// Attributes that make a C# method a test in xUnit (`[Fact]`, `[Theory]`),
/// NUnit (`[Test]`, `[TestCase]`, `[TestCaseSource]`) or MSTest
/// (`[TestMethod]`, `[DataTestMethod]`), and classes a runner collects
/// (`[TestFixture]`, `[TestClass]`).
const CSHARP_TEST_ATTRIBUTES: [&str; 7] = [
    "Fact",
    "Theory",
    "Test",
    "TestCase",
    "TestCaseSource",
    "TestMethod",
    "DataTestMethod",
];
const CSHARP_TEST_CLASS_ATTRIBUTES: [&str; 2] = ["TestFixture", "TestClass"];

/// The names of the attributes on a C# declaration, without a namespace or
/// the `Attribute` suffix: `[Xunit.FactAttribute]` is `Fact`.
fn csharp_attributes<'a>(node: Node<'_>, source: &'a str) -> Vec<&'a str> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for list in node
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "attribute_list")
    {
        let mut inner = list.walk();
        for attribute in list
            .named_children(&mut inner)
            .filter(|c| c.kind() == "attribute")
        {
            if let Some(name) = attribute.child_by_field_name("name") {
                let name = child_text(name, source);
                let name = name.rsplit('.').next().unwrap_or(name);
                let name = name.split('<').next().unwrap_or(name);
                names.push(name.strip_suffix("Attribute").unwrap_or(name));
            }
        }
    }
    names
}

/// A C# test method: one with a test attribute of a supported framework.
pub(crate) fn csharp_test_method(node: Node<'_>, source: &str) -> bool {
    node.kind() == "method_declaration"
        && csharp_attributes(node, source)
            .iter()
            .any(|name| CSHARP_TEST_ATTRIBUTES.contains(name))
}

/// A C# test class: marked as a fixture, or holding test methods. Its setup,
/// fields and helpers are test code as well.
pub(crate) fn csharp_test_class(node: Node<'_>, source: &str) -> bool {
    if !matches!(node.kind(), "class_declaration" | "record_declaration") {
        return false;
    }
    if csharp_attributes(node, source)
        .iter()
        .any(|name| CSHARP_TEST_CLASS_ATTRIBUTES.contains(name))
    {
        return true;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .any(|member| csharp_test_method(member, source))
}

/// A C# test class, whole; nested test classes are inside its span.
fn csharp_test_span(node: Node<'_>, source: &str) -> Option<(usize, usize)> {
    csharp_test_class(node, source).then(|| (node.start_byte(), node.end_byte()))
}

/// A Ruby example group or example written as a statement with a block:
/// RSpec's `describe Order do`, `RSpec.describe`, `context`, `it "adds" do`
/// and `it { is_expected.to … }`, and a Rails `test "adds" do`. An example
/// is titled by a string or not at all, so `test(:unit) do` in a Rakefile
/// is not one.
pub(crate) fn ruby_test_call(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "call" || node.child_by_field_name("block").is_none() {
        return false;
    }
    let method = ruby::method(node, source);
    let receiver = node
        .child_by_field_name("receiver")
        .map(|r| child_text(r, source));
    let statement = node
        .parent()
        .is_some_and(|p| matches!(p.kind(), "program" | "body_statement" | "block_body"));
    let titled = || match ruby::first_argument(node) {
        None => true,
        Some(title) => {
            title.kind() == "string" || (method == "its" && title.kind() == "simple_symbol")
        }
    };
    statement
        && receiver.is_none_or(|r| r == "RSpec")
        && (ruby::GROUPS.contains(&method) || ruby::CASES.contains(&method) && titled())
}

/// A Minitest, Test::Unit or Rails test class: its superclass ends in `Test`,
/// `TestCase` or `Spec`, as `Minitest::Test`, `ActiveSupport::TestCase` and
/// `ActionDispatch::IntegrationTest` do.
pub(crate) fn ruby_test_class(node: Node<'_>, source: &str) -> bool {
    node.kind() == "class"
        && node
            .child_by_field_name("superclass")
            .and_then(|s| s.named_child(0))
            .is_some_and(|base| {
                let name = child_text(base, source).rsplit("::").next().unwrap_or("");
                name.ends_with("Test") || name.ends_with("TestCase") || name == "Spec"
            })
}

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
fn python_test_span(node: Node<'_>, source: &str, pytest: bool) -> Option<(usize, usize)> {
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

fn rust_test_span(node: Node<'_>, source: &str) -> Option<(usize, usize)> {
    if !matches!(
        node.kind(),
        "function_item" | "function_signature_item" | "mod_item" | "impl_item"
    ) {
        return None;
    }
    let marked = preceding_attributes(node, source)
        .iter()
        .any(|text| attribute_marks_test(text))
        || has_inner_cfg_test(node, source)
        || mod_contains_only_tests(node, source);
    if !marked {
        return None;
    }
    Some((attribute_start(node), node.end_byte()))
}

fn javascript_test_span(node: Node<'_>, source: &str) -> Option<(usize, usize)> {
    if node.kind() != "call_expression" || !is_test_call(&callee(node, source)) {
        return None;
    }
    let statement = statement_span(node)?;
    Some((statement.start_byte(), statement.end_byte()))
}

fn mod_contains_only_tests(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "mod_item" {
        return false;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut saw_test = false;
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind().contains("comment")
            || matches!(
                child.kind(),
                "attribute_item" | "inner_attribute_item" | "use_declaration"
            )
        {
            continue;
        }
        if rust_test_span(child, source).is_some() {
            saw_test = true;
            continue;
        }
        return false;
    }
    saw_test
}

fn whole_file_cfg_test(root: Node<'_>, source: &str) -> bool {
    let mut cursor = root.walk();
    root.named_children(&mut cursor).any(|child| {
        child.kind() == "inner_attribute_item" && cfg_is_test_only(child_text(child, source))
    })
}

fn has_inner_cfg_test(node: Node<'_>, source: &str) -> bool {
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    body.named_children(&mut cursor).any(|child| {
        child.kind() == "inner_attribute_item" && cfg_is_test_only(child_text(child, source))
    })
}

pub(crate) fn preceding_attributes<'a>(node: Node<'_>, source: &'a str) -> Vec<&'a str> {
    let mut texts = Vec::new();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() == "attribute_item" {
            texts.push(child_text(sibling, source));
        } else if !sibling.kind().contains("comment") {
            break;
        }
        previous = sibling.prev_named_sibling();
    }
    texts
}

fn attribute_start(node: Node<'_>) -> usize {
    let mut start = node.start_byte();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() == "attribute_item" {
            start = sibling.start_byte();
        } else if !sibling.kind().contains("comment") {
            break;
        }
        previous = sibling.prev_named_sibling();
    }
    start
}

pub(crate) fn attribute_marks_test(text: &str) -> bool {
    cfg_is_test_only(text) || attribute_path(text).is_some_and(is_test_attribute)
}

fn attribute_path(text: &str) -> Option<&str> {
    let start = text.find('[')? + 1;
    let end = text.rfind(']')?;
    let body = text.get(start..end)?.trim();
    let path = body.split(['(', '=']).next()?.trim();
    (!path.is_empty()).then_some(path)
}

fn is_test_attribute(path: &str) -> bool {
    path == "test"
        || path.ends_with("::test")
        || path == "rstest"
        || path.ends_with("::rstest")
        || path == "test_case"
        || path.ends_with("::test_case")
}

/// `cfg(test)` and `all(..., test, ...)` compile only for tests.
/// `not(test)` and `any(test, ...)` can still be production code, so they stay.
fn cfg_is_test_only(text: &str) -> bool {
    let cleaned = strip_strings(text);
    let mut rest = cleaned.as_str();
    while let Some(index) = rest.find("cfg") {
        let boundary = index == 0
            || !rest.as_bytes()[index - 1].is_ascii_alphanumeric()
                && rest.as_bytes()[index - 1] != b'_';
        let after = rest[index + 3..].trim_start();
        if boundary
            && let Some(body) = after.strip_prefix('(')
            && let Some(end) = matching_paren(body)
            && predicate_is_test_only(&body[..end])
        {
            return true;
        }
        rest = &rest[index + 3..];
    }
    false
}

fn predicate_is_test_only(expr: &str) -> bool {
    let expr = expr.trim();
    if expr == "test" {
        return true;
    }
    strip_call(expr, "all").is_some_and(|inner| {
        split_top_level(inner)
            .iter()
            .any(|part| predicate_is_test_only(part))
    })
}

fn strip_call<'a>(expr: &'a str, name: &str) -> Option<&'a str> {
    let rest = expr.trim().strip_prefix(name)?.trim_start();
    let rest = rest.strip_prefix('(')?;
    let end = matching_paren(rest)?;
    rest[end..].trim().is_empty().then_some(rest[..end].trim())
}

fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                let part = text[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn matching_paren(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(index),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn strip_strings(text: &str) -> String {
    let mut cleaned = String::new();
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '"' || character == '\'' {
            let quote = character;
            while let Some(next) = chars.next() {
                if next == '\\' {
                    chars.next();
                    continue;
                }
                if next == quote {
                    break;
                }
            }
            cleaned.push(' ');
        } else {
            cleaned.push(character);
        }
    }
    cleaned
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

fn child_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn owns_lines(source: &str, start: usize, end: usize) -> bool {
    if start > end || end > source.len() {
        return false;
    }
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[end..]
        .find('\n')
        .map_or(source.len(), |index| end + index);
    source[line_start..start].trim().is_empty() && source[end..line_end].trim().is_empty()
}

fn line_range(source: &str, start: usize, end: usize) -> SourceRange {
    let start_line = source[..start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let end_at = end.saturating_sub(1).max(start);
    let end_line = source[..end_at.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    SourceRange {
        start_line,
        end_line,
    }
}

fn merge_spans(mut spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for span in spans {
        if let Some(last) = merged.last_mut()
            && span.0 <= last.1
        {
            last.1 = last.1.max(span.1);
            continue;
        }
        merged.push(span);
    }
    merged
}
