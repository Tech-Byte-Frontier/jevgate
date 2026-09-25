//! Ruby definitions made by calls and assignments rather than `def`:
//! `define_method`, blocks registered by a call (`get('/invoices') do … end`),
//! and constants bound to a lambda or to a class built with a block; and the
//! files a `require` imports.
use super::{Definition, FileUnits, Kind, children, push, walk};
use crate::analysis::{ruby, text};
use tree_sitter::Node;

/// A Ruby call at the level of a file, class or module: a `require` names an
/// import; `define_method(:name) do … end` defines a method; a block that
/// holds definitions (`helpers do`, `included do`) is read for them; test
/// declarations are read only for the definitions inside them; and another
/// call with a block registers it, named by its call: `get('/')`.
pub(super) fn ruby_call(node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    if let Some(path) = ruby::required(node, source) {
        let stem = path.rsplit('/').next().unwrap_or(path);
        let stem = stem.strip_suffix(".rb").unwrap_or(stem);
        if !stem.is_empty() {
            file.imports.insert(stem.to_string());
        }
        return;
    }
    let Some(body) = ruby::block_body(node) else {
        // `private def total` and `memoize def rates` define a method.
        if let Some(arguments) = node.child_by_field_name("arguments") {
            let mut cursor = arguments.walk();
            for argument in arguments.named_children(&mut cursor) {
                if matches!(argument.kind(), "method" | "singleton_method") {
                    walk(argument, source, owner, file);
                }
            }
        }
        return;
    };
    let method = ruby::method(node, source);
    if method == "define_method"
        && let Some(name) = ruby::first_argument(node).filter(|a| a.kind() == "simple_symbol")
    {
        let definition = Definition {
            outer: node,
            node,
            body: Some(body),
        };
        let name = text(name, source).trim_start_matches(':');
        let kind = if owner.is_empty() {
            Kind::Function
        } else {
            Kind::Method
        };
        push(definition, name, owner, kind, source, file);
        return;
    }
    if ruby::test_dsl(node, source) {
        ruby_block_definitions(body, source, owner, file);
        return;
    }
    if ruby::defines(body) {
        children(body, source, owner, file);
        return;
    }
    let receiver = node
        .child_by_field_name("receiver")
        .map(|r| format!("{}.", text(r, source)))
        .unwrap_or_default();
    let argument = match ruby::first_argument(node) {
        Some(first)
            if matches!(
                first.kind(),
                "string" | "simple_symbol" | "constant" | "scope_resolution"
            ) =>
        {
            format!("({})", text(first, source))
        }
        Some(_) => "(…)".into(),
        None => String::new(),
    };
    let definition = Definition {
        outer: node,
        node,
        body: Some(body),
    };
    let name = format!("{receiver}{method}{argument}");
    push(definition, &name, owner, Kind::Function, source, file);
}

/// Definitions inside the blocks of Ruby test declarations, such as a helper
/// method in a `describe` block; the blocks themselves are left to the test rules.
fn ruby_block_definitions(body: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "method" | "singleton_method" | "class" | "module" | "singleton_class" => {
                walk(child, source, owner, file);
            }
            "call" => {
                if let Some(inner) = ruby::block_body(child) {
                    ruby_block_definitions(inner, source, owner, file);
                }
            }
            _ => {}
        }
    }
}

/// A Ruby constant bound to a lambda (`ROUND = ->(value) { … }`) is a
/// function; one bound to a class built with a block (`Point = Struct.new(:x)
/// do … end`) owns the methods of the block.
pub(super) fn ruby_assignment(node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    let (Some(left), Some(right)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return;
    };
    if !matches!(left.kind(), "constant" | "identifier") {
        return;
    }
    let name = text(left, source);
    if right.kind() == "lambda" {
        let definition = Definition {
            outer: node,
            node: right,
            body: right
                .child_by_field_name("body")
                .and_then(|b| b.child_by_field_name("body")),
        };
        push(definition, name, owner, Kind::Function, source, file);
    } else if let Some(body) = ruby::block_body(right).filter(|b| ruby::defines(*b)) {
        children(body, source, name, file);
    }
}
