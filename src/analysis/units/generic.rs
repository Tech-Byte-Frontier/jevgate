//! Units of a language of the generic tier (`analysis::generic`): the
//! definitions its tag query captures, each with what function
//! simplification, file organization, shared logic and comments read. A
//! definition inside a function is part of that function's code. A function
//! inside a type is a method of the innermost one, or of the type or table
//! its name is written in (`Cart::add`, `M.add`). A type that holds no
//! function is a unit, the outermost of nested ones: `typedef struct erow
//! {…} erow;` is one type, not two.
use super::{Definition, FileUnits, Kind, Unit};
use crate::analysis::{
    blocks,
    generic::{self, Defines, Language, Tag},
    is_comment, nesting, text,
};
use std::collections::BTreeSet;
use tree_sitter::Node;

pub(super) fn walk(language: &Language, root: Node<'_>, source: &str, file: &mut FileUnits) {
    let tags = generic::tags(language, root, source);
    let (functions, types): (Vec<&Tag<'_>>, Vec<&Tag<'_>>) = tags
        .definitions
        .iter()
        .filter(|tag| !under_error(tag.node))
        .partition(|tag| tag.defines == Defines::Function);
    let in_function = |tag: &Tag<'_>| functions.iter().any(|f| inside(f.node, tag.node));
    let functions: Vec<&Tag<'_>> = functions
        .iter()
        .copied()
        .filter(|f| !in_function(f))
        .collect();
    let types: Vec<&Tag<'_>> = types.into_iter().filter(|t| !in_function(t)).collect();
    let holds_function = |tag: &Tag<'_>| functions.iter().any(|f| inside(tag.node, f.node));
    for function in &functions {
        let owner = types
            .iter()
            .filter(|t| inside(t.node, function.node))
            .max_by_key(|t| t.node.start_byte())
            .map(|t| t.name)
            .or(function.scope)
            .map_or("", |name| text(name, source));
        // Elixir's function head, `add(cart, item)`, is a call of the
        // function's own name.
        let calls = tags
            .calls
            .iter()
            .filter(|name| inside(function.node, **name) && name.id() != function.name.id())
            .map(|name| text(*name, source).to_string())
            .collect();
        push(language, function, (owner, calls), source, file);
    }
    for tag in &types {
        let nested_in_type = types
            .iter()
            .any(|outer| inside(outer.node, tag.node) && !holds_function(outer));
        if !holds_function(tag) && !nested_in_type {
            push(language, tag, ("", BTreeSet::new()), source, file);
        }
    }
    file.units.sort_by_key(|unit| unit.span.start);
}

/// Whether `inner` lies within `outer` and is not `outer` itself.
fn inside(outer: Node<'_>, inner: Node<'_>) -> bool {
    outer.id() != inner.id()
        && outer.start_byte() <= inner.start_byte()
        && inner.end_byte() <= outer.end_byte()
}

/// Whether a node lies in what the parser could not read: queries match
/// there too, where the other languages' walks never look.
fn under_error(node: Node<'_>) -> bool {
    std::iter::successors(node.parent(), Node::parent).any(|n| n.is_error())
}

/// One definition's unit, with its owner and the names it calls. One that
/// holds a syntax error is left out, as in every language.
fn push(
    language: &Language,
    tag: &Tag<'_>,
    (owner, calls): (&str, BTreeSet<String>),
    source: &str,
    file: &mut FileUnits,
) {
    let short_name = text(tag.name, source);
    if short_name.is_empty() || tag.node.has_error() {
        return;
    }
    let kind = match (tag.defines, owner.is_empty()) {
        (Defines::Type, _) => Kind::Type,
        (Defines::Function, true) => Kind::Function,
        (Defines::Function, false) => Kind::Method,
    };
    let body = (kind != Kind::Type).then(|| body(tag, language)).flatten();
    let definition = Definition {
        outer: tag.node,
        node: tag.node,
        body,
    };
    let (nesting, branch_chain) = body.map_or((0, 0), |b| nesting::generic(b, language));
    let mut refs = BTreeSet::from([short_name.to_string()]);
    identifiers(tag.node, source, &|_| true, &mut refs);
    if !owner.is_empty() {
        refs.insert(owner.to_string());
    }
    // The values its body names, among them functions it passes by name;
    // types (`type_identifier`) and fields are not called.
    let mut mentions = BTreeSet::new();
    if let Some(body) = body {
        let value = |kind: &str| matches!(kind, "identifier" | "simple_identifier");
        identifiers(body, source, &value, &mut mentions);
    }
    let unit = Unit {
        nesting,
        branch_chain,
        blocks: body.map_or_else(Vec::new, |b| blocks::blocks_in(b, source, language.blocks)),
        calls,
        refs,
        mentions,
        ..Unit::placed(definition, (short_name, owner), kind, source)
    };
    file.units.push(unit);
}

/// The node holding a definition's statements: its `@body` capture or
/// `body` field, read through a wrapper around them, as Swift's
/// `function_body` wraps its `statements`. A body written as an expression
/// is the expression.
fn body<'t>(tag: &Tag<'t>, language: &Language) -> Option<Node<'t>> {
    let body = tag.body.or_else(|| tag.node.child_by_field_name("body"))?;
    if language.blocks.contains(&body.kind()) || body.named_child_count() != 1 {
        return Some(body);
    }
    Some(
        body.named_child(0)
            .filter(|inner| language.blocks.contains(&inner.kind()))
            .unwrap_or(body),
    )
}

/// The identifiers under `node` outside comments whose kind `keep` accepts:
/// the names of the types, fields and functions it uses. Elixir names
/// modules with aliases.
fn identifiers(
    node: Node<'_>,
    source: &str,
    keep: &dyn Fn(&str) -> bool,
    found: &mut BTreeSet<String>,
) {
    if is_comment(node) {
        return;
    }
    let kind = node.kind();
    if kind.ends_with("identifier") || kind == "alias" {
        if keep(kind) {
            found.insert(text(node, source).to_string());
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        identifiers(child, source, keep, found);
    }
}
