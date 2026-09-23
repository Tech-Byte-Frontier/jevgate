//! Symbol locations and token-normalized content fingerprints of one file.
use crate::syntax::parse;
use anyhow::Result;
use std::path::Path;
use tree_sitter::Node;

/// A file with more functions than this is located by line windows instead.
const MAX_LOCATIONS: usize = 256;
/// Line windows: at most this many, each at least `MIN_WINDOW_LINES` long.
const MAX_WINDOWS: usize = 128;
const MIN_WINDOW_LINES: usize = 80;

fn visit(node: Node<'_>, source: &str, locations: &mut Vec<(String, usize)>) {
    if matches!(
        node.kind(),
        "function_item"
            | "function_definition"
            | "function_declaration"
            | "method_definition"
            | "arrow_function"
            | "function_expression"
    ) {
        locations.push((function_name(node, source), node.start_position().row + 1));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, source, locations);
    }
}

pub fn collect(path: &Path, source: &str, _root: &Path) -> Result<(bool, Vec<(String, usize)>)> {
    let tree = parse(path, source)?;
    if let Some(tree) = &tree {
        let mut locations = Vec::new();
        visit(tree.root_node(), source, &mut locations);
        if locations.len() <= MAX_LOCATIONS {
            return Ok((true, locations));
        }
    }
    Ok((tree.is_some(), line_windows(source)))
}

/// The function's own name, or the name of the binding that holds it.
fn function_name(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("name")
        .or_else(|| node.parent().and_then(|p| p.child_by_field_name("name")))
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .unwrap_or("anonymous")
        .to_owned()
}

/// Fixed line ranges, used when a file has no parser or too many functions.
fn line_windows(source: &str) -> Vec<(String, usize)> {
    let lines = source.lines().count().max(1);
    let step = lines.div_ceil(MAX_WINDOWS).max(MIN_WINDOW_LINES);
    (1..=lines)
        .step_by(step)
        .map(|start| {
            (
                format!("Lines {start}-{}", (start + step - 1).min(lines)),
                start,
            )
        })
        .collect()
}

fn tokens(node: Node<'_>, source: &str, out: &mut String) {
    if node.kind().contains("comment") {
        return;
    }
    if node.child_count() == 0 {
        if node.kind().contains("identifier") {
            out.push_str("IDENT");
        } else {
            out.push_str(node.utf8_text(source.as_bytes()).unwrap_or_default());
        }
        out.push('\0');
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            tokens(child, source, out);
        }
    }
}

fn token_text(path: &Path, source: &str) -> Option<String> {
    let tree = parse(path, source).ok().flatten()?;
    let mut normalized = String::new();
    tokens(tree.root_node(), source, &mut normalized);
    Some(normalized)
}

pub fn identity(path: &Path, source: &str) -> String {
    let text = token_text(path, source).unwrap_or_else(|| source.to_string());
    crate::schema::hash(text.as_bytes())
}

pub fn semantic_size(path: &Path, source: &str) -> usize {
    token_text(path, source)
        .map(|text| text.bytes().filter(|byte| *byte == 0).count())
        .unwrap_or_else(|| source.split_whitespace().count())
}
