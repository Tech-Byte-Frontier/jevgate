//! Locate complete definitions and their original source ranges.
use crate::schema::SourceRange;
use std::{collections::BTreeSet, ops::Range, path::Path};
use tree_sitter::Node;

struct Unit {
    span: Range<usize>,
    names: BTreeSet<String>,
    reviewable: bool,
    owner: String,
}

fn identifiers(node: Node<'_>, source: &str, result: &mut BTreeSet<String>) {
    if node.kind().contains("comment") || node.kind().contains("string") {
        return;
    }
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "field_identifier" | "constant"
    ) {
        result.insert(node.utf8_text(source.as_bytes()).unwrap_or("").into());
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        identifiers(child, source, result);
    }
}

/// A string literal that is the first statement of a module or class body.
fn python_docstring(node: Node<'_>, source: &str) -> bool {
    node.kind() == "expression_statement"
        && docstring_position(node)
        && node.named_child_count() == 1
        && node
            .named_child(0)
            .is_some_and(|child| string_literal(child, source))
}

/// Directly in a module or class body, after nothing but comments.
fn docstring_position(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    let body = parent.kind() == "module"
        || (parent.kind() == "block"
            && parent
                .parent()
                .is_some_and(|p| p.kind() == "class_definition"));
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if !sibling.kind().contains("comment") {
            return false;
        }
        previous = sibling.prev_named_sibling();
    }
    body
}

/// A plain (not bytes or f-) string, possibly parenthesized or concatenated.
fn string_literal(node: Node<'_>, source: &str) -> bool {
    match node.kind() {
        "string" => !node
            .utf8_text(source.as_bytes())
            .unwrap_or("")
            .chars()
            .take_while(|c| !matches!(c, '\'' | '"'))
            .any(|c| matches!(c, 'b' | 'B' | 'f' | 'F')),
        "parenthesized_expression" | "concatenated_string" => {
            let mut cursor = node.walk();
            let mut children = node
                .named_children(&mut cursor)
                .filter(|child| !child.kind().contains("comment"));
            children
                .next()
                .is_some_and(|child| string_literal(child, source))
                && children.all(|child| string_literal(child, source))
        }
        _ => false,
    }
}

fn collect(
    node: Node<'_>,
    source: &str,
    units: &mut Vec<Unit>,
    scaffolding: &mut Vec<Range<usize>>,
) {
    let kind = node.kind();
    if matches!(
        kind,
        "use_declaration"
            | "import_statement"
            | "import_from_statement"
            | "inner_attribute_item"
            | "using_directive"
            | "file_scoped_namespace_declaration"
    ) || python_docstring(node, source)
        || crate::analysis::ruby::required(node, source).is_some()
    {
        scaffolding.push(definition_start(node)..node.end_byte());
        return;
    }
    if kind == "attribute_item" {
        return;
    }
    if kind.contains("comment") {
        return;
    }
    if matches!(
        kind,
        "source_file"
            | "program"
            | "module"
            | "declaration_list"
            | "class_body"
            | "block"
            | "compilation_unit"
    ) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect(child, source, units, scaffolding);
        }
        return;
    }
    if container(node, source, units, scaffolding) {
        return;
    }
    let names = unit_names(node, source);
    let enclosing = enclosing_owner(node, source);
    // Preserve documentation and decorators that immediately precede this unit.
    units.push(Unit {
        span: definition_start(node)..node.end_byte(),
        names,
        owner: enclosing,
        reviewable: !matches!(
            kind,
            "struct_item"
                | "enum_item"
                | "type_item"
                | "trait_item"
                | "mod_item"
                | "interface_declaration"
                | "type_alias_declaration"
        ),
    });
}

/// Keep the enclosing module/impl/class declaration and select complete members.
fn container(
    node: Node<'_>,
    source: &str,
    units: &mut Vec<Unit>,
    scaffolding: &mut Vec<Range<usize>>,
) -> bool {
    let csharp = matches!(
        node.kind(),
        "namespace_declaration"
            | "struct_declaration"
            | "record_declaration"
            | "interface_declaration"
    );
    if !csharp
        && !matches!(
            node.kind(),
            "impl_item" | "mod_item" | "class_declaration" | "class_definition" | "singleton_class"
        )
        && !ruby_namespace(node)
    {
        return false;
    }
    let Some(body) = node
        .child_by_field_name("body")
        .filter(|body| !csharp || body.kind() == "declaration_list")
    else {
        return false;
    };
    let first = body
        .named_child(0)
        .map_or(body.end_byte(), |n| n.start_byte());
    let last = body
        .named_child(body.named_child_count().saturating_sub(1) as u32)
        .map_or(first, |n| n.end_byte());
    scaffolding.push(definition_start(node)..first);
    scaffolding.push(last..node.end_byte());
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        collect(child, source, units, scaffolding);
    }
    true
}

/// A Ruby `module` or `class`, whose name is a constant.
fn ruby_namespace(node: Node<'_>) -> bool {
    matches!(node.kind(), "module" | "class")
        && node
            .child_by_field_name("name")
            .is_some_and(|n| matches!(n.kind(), "constant" | "scope_resolution"))
}

/// Names a definition binds. Bindings and export wrappers may name the
/// definition below their root.
fn unit_names(node: Node<'_>, source: &str) -> BTreeSet<String> {
    fn add(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
        for field in ["name", "left"] {
            if let Some(name) = node.child_by_field_name(field) {
                identifiers(name, source, names);
            }
        }
    }
    let mut names = BTreeSet::new();
    add(node, source, &mut names);
    if names.is_empty()
        && matches!(
            node.kind(),
            "export_statement"
                | "lexical_declaration"
                | "variable_declaration"
                | "expression_statement"
                | "decorated_definition"
        )
    {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            add(child, source, &mut names);
        }
    }
    names
}

/// The nearest impl or class that contains `node`, by name.
fn enclosing_owner(node: Node<'_>, source: &str) -> String {
    let mut owner = node.parent();
    while let Some(parent) = owner {
        if matches!(
            parent.kind(),
            "impl_item"
                | "class_declaration"
                | "class_definition"
                | "struct_declaration"
                | "record_declaration"
                | "interface_declaration"
        ) || ruby_namespace(parent)
        {
            return parent
                .child_by_field_name("type")
                .or_else(|| parent.child_by_field_name("name"))
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("")
                .into();
        }
        owner = parent.parent();
    }
    String::new()
}

fn definition_start(node: Node<'_>) -> usize {
    let mut start = node.start_byte();
    let mut preceding = node.prev_named_sibling();
    while let Some(previous) = preceding {
        if !(previous.kind().contains("comment") || previous.kind() == "attribute_item") {
            break;
        }
        start = previous.start_byte();
        preceding = previous.prev_named_sibling();
    }
    start
}

fn line_number(source: &str, byte: usize) -> usize {
    source[..byte].bytes().filter(|b| *b == b'\n').count() + 1
}

pub fn review_targets(path: &Path, source: &str) -> Vec<(String, SourceRange, String)> {
    let Some(tree) = crate::syntax::parse(path, source).ok().flatten() else {
        return Vec::new();
    };
    let mut units = Vec::new();
    collect(tree.root_node(), source, &mut units, &mut Vec::new());
    units
        .into_iter()
        .filter(|unit| unit.reviewable)
        .map(|unit| {
            let range = SourceRange {
                start_line: line_number(source, unit.span.start),
                end_line: line_number(source, unit.span.end),
            };
            let name = target_name(&unit, range.start_line);
            (name, range, source[unit.span].to_owned())
        })
        .collect()
}

/// The unit's names, qualified by its owner; unnamed code by its line.
fn target_name(unit: &Unit, line: usize) -> String {
    let name = if unit.names.is_empty() {
        format!("declaration/effect at line {line}")
    } else {
        unit.names.iter().cloned().collect::<Vec<_>>().join(", ")
    };
    if unit.owner.is_empty() {
        name
    } else {
        format!("{}::{name}", unit.owner)
    }
}
