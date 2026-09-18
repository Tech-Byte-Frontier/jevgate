//! Locate complete definitions and their original source ranges.
use crate::{locations, schema::SourceRange};
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
        "identifier" | "type_identifier" | "field_identifier"
    ) {
        result.insert(node.utf8_text(source.as_bytes()).unwrap_or("").into());
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        identifiers(child, source, result);
    }
}

fn python_docstring(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "expression_statement" {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != "module"
        && !(parent.kind() == "block"
            && parent
                .parent()
                .is_some_and(|p| p.kind() == "class_definition"))
    {
        return false;
    }
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if !sibling.kind().contains("comment") {
            return false;
        }
        previous = sibling.prev_named_sibling();
    }
    fn literal(node: Node<'_>, source: &str) -> bool {
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
                children.next().is_some_and(|child| literal(child, source))
                    && children.all(|child| literal(child, source))
            }
            _ => false,
        }
    }
    node.named_child_count() == 1
        && node
            .named_child(0)
            .is_some_and(|child| literal(child, source))
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
        "use_declaration" | "import_statement" | "import_from_statement" | "inner_attribute_item"
    ) || python_docstring(node, source)
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
        "source_file" | "program" | "module" | "declaration_list" | "class_body" | "block"
    ) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect(child, source, units, scaffolding);
        }
        return;
    }
    // Keep the enclosing module/impl/class declaration and select complete members.
    if matches!(
        kind,
        "impl_item" | "mod_item" | "class_declaration" | "class_definition"
    ) && let Some(body) = node.child_by_field_name("body")
    {
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
        return;
    }
    let mut names = BTreeSet::new();
    if let Some(name) = node.child_by_field_name("name") {
        identifiers(name, source, &mut names);
    }
    if let Some(name) = node.child_by_field_name("left") {
        identifiers(name, source, &mut names);
    }
    // Bindings and export wrappers may name the definition below their root.
    if names.is_empty()
        && matches!(
            kind,
            "export_statement"
                | "lexical_declaration"
                | "variable_declaration"
                | "expression_statement"
                | "decorated_definition"
        )
    {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(name) = child.child_by_field_name("name") {
                identifiers(name, source, &mut names);
            }
            if let Some(name) = child.child_by_field_name("left") {
                identifiers(name, source, &mut names);
            }
        }
    }
    let mut owner = node.parent();
    let mut enclosing = String::new();
    while let Some(parent) = owner {
        if enclosing.is_empty()
            && matches!(
                parent.kind(),
                "impl_item" | "class_declaration" | "class_definition"
            )
        {
            enclosing = parent
                .child_by_field_name("type")
                .or_else(|| parent.child_by_field_name("name"))
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("")
                .into();
        }
        owner = parent.parent();
    }
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

pub fn review_targets(path: &Path, source: &str) -> Vec<(String, SourceRange, String)> {
    let Some(tree) = locations::parse(path, source).ok().flatten() else {
        return Vec::new();
    };
    let mut units = Vec::new();
    collect(tree.root_node(), source, &mut units, &mut Vec::new());
    units
        .into_iter()
        .filter(|unit| unit.reviewable)
        .map(|unit| {
            let range = SourceRange {
                start_line: source[..unit.span.start]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count()
                    + 1,
                end_line: source[..unit.span.end]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count()
                    + 1,
            };
            let name = if unit.names.is_empty() {
                format!("declaration/effect at line {}", range.start_line)
            } else {
                unit.names.into_iter().collect::<Vec<_>>().join(", ")
            };
            let name = if unit.owner.is_empty() {
                name
            } else {
                format!("{}::{name}", unit.owner)
            };
            (name, range, source[unit.span].to_owned())
        })
        .collect()
}
