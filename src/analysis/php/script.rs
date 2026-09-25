//! A PHP page script: the top-level statements that run when the file is
//! requested, and the text they write.
use crate::analysis::text;
use tree_sitter::Node;

/// Top-level statements that define or import rather than run.
const DEFINITIONS: &[&str] = &[
    "php_tag",
    "php_end_tag",
    "text_interpolation",
    "text",
    "function_definition",
    "class_declaration",
    "interface_declaration",
    "trait_declaration",
    "enum_declaration",
    "namespace_use_declaration",
    "declare_statement",
    "const_declaration",
    "empty_statement",
];

/// Top-level statements of a PHP file that run when it is requested or
/// included, inside namespace blocks too: everything but definitions,
/// imports, inline HTML and statements holding a unit.
pub(crate) fn script_statements<'t>(root: Node<'t>, found: &mut Vec<Node<'t>>) {
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if crate::analysis::is_comment(node) || DEFINITIONS.contains(&node.kind()) {
            continue;
        }
        if node.kind() == "namespace_definition" {
            if let Some(body) = node.child_by_field_name("body") {
                script_statements(body, found);
            }
            continue;
        }
        found.push(node);
    }
}

/// `<?= $value ?>`: an expression statement that a short echo tag opens.
pub(crate) fn short_echo(node: Node<'_>, source: &str) -> bool {
    node.kind() == "expression_statement"
        && node.prev_sibling().is_some_and(|previous| {
            let tag = if previous.kind() == "text_interpolation" {
                previous.named_children(&mut previous.walk()).last()
            } else {
                Some(previous)
            };
            tag.is_some_and(|t| t.kind() == "php_tag" && text(t, source) == "<?=")
        })
}

/// A double-quoted string or heredoc that interpolates a value.
pub(crate) fn interpolates(node: Node<'_>) -> bool {
    let parts = match node.kind() {
        "encapsed_string" => Some(node),
        "heredoc" => node.child_by_field_name("value"),
        _ => None,
    };
    parts.is_some_and(|parts| {
        let mut cursor = parts.walk();
        parts.named_children(&mut cursor).any(|c| {
            !matches!(
                c.kind(),
                "string_content" | "escape_sequence" | "heredoc_start" | "heredoc_end"
            )
        })
    })
}
