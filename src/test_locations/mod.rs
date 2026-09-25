//! Structural tests a parser can locate: Rust test attributes and `cfg(test)`
//! modules, JavaScript and TypeScript `describe`/`it`/`test` calls, Python
//! test classes and pytest functions, Go `Test…(t *testing.T)` functions, C#
//! classes of xUnit, NUnit or MSTest tests, Ruby RSpec groups and examples
//! and Minitest or Rails test classes, PHPUnit `TestCase` classes and Pest
//! `test`/`it` calls, and Java classes holding JUnit or TestNG tests. Syntax
//! locates tests; it never judges them.
mod csharp;
mod java;
mod javascript;
mod python;
mod ruby;
mod rust;

use crate::schema::SourceRange;
use anyhow::Result;
pub(crate) use csharp::csharp_test_method;
use csharp::csharp_test_span;
use java::java_test_class;
pub(crate) use java::{java_test_method, junit3_class};
use javascript::javascript_test_span;
use python::python_test_span;
pub(crate) use python::{pytest_file, python_test_class};
pub(crate) use ruby::{ruby_test_call, ruby_test_class};
pub(crate) use rust::{attribute_marks_test, preceding_attributes};
use rust::{rust_test_span, whole_file_cfg_test};
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
        .or_else(|| php_test_span(node, source))
        .or_else(|| java_test_class(node, source).then(|| (node.start_byte(), node.end_byte())))
    {
        spans.push(span);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, source, pytest, spans);
    }
}

/// A PHPUnit test class with the comments above it, or a Pest statement.
fn php_test_span(node: Node<'_>, source: &str) -> Option<(usize, usize)> {
    use crate::analysis::php;
    let test = php::test_class(node, source) || php::pest_statement(node, source).is_some();
    test.then(|| {
        let mut start = node.start_byte();
        let mut previous = node.prev_named_sibling();
        while let Some(comment) = previous.filter(|p| p.kind() == "comment") {
            start = comment.start_byte();
            previous = comment.prev_named_sibling();
        }
        (start, node.end_byte())
    })
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
