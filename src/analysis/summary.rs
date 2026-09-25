//! What an outline shows of a definition: its header on one line and the
//! first line of its documentation, both clipped.
use super::text;
use tree_sitter::Node;

const SIGNATURE_CHARS: usize = 240;
const DOC_CHARS: usize = 160;

/// The definition's header on one line, without attributes, decorators or
/// the opening of its body. C# attribute lists are children of the
/// declaration, so the header starts after them.
pub(super) fn signature(outer: Node<'_>, body: Option<Node<'_>>, source: &str) -> String {
    let end = body.map_or_else(|| ruby_header_end(outer), |b| b.start_byte());
    let mut cursor = outer.walk();
    let start = outer
        .named_children(&mut cursor)
        .take_while(|c| c.kind() == "attribute_list" || c.kind().contains("comment"))
        .last()
        .map_or(outer.start_byte(), |c| c.end_byte())
        .min(end);
    clip(
        source[start..end]
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("#[") && !line.starts_with('@'))
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end_matches(['{', ':', ' ', '='])
            .trim_end_matches("=>")
            .trim(),
        SIGNATURE_CHARS,
    )
}

/// Where a definition without a body ends its header: a Ruby `def` at its
/// parameters or name, and a Ruby `class` or `module` at its superclass or
/// name, before the statements and the closing `end`. Other definitions end
/// where they end.
fn ruby_header_end(node: Node<'_>) -> usize {
    let constant = || {
        node.child_by_field_name("name")
            .is_some_and(|n| matches!(n.kind(), "constant" | "scope_resolution"))
    };
    let fields: &[&str] = match node.kind() {
        "method" | "singleton_method" => &["parameters", "name"],
        "class" if constant() => &["superclass", "name"],
        "module" if constant() => &["name"],
        _ => &[],
    };
    fields
        .iter()
        .find_map(|field| node.child_by_field_name(field))
        .map_or(node.end_byte(), |n| n.end_byte())
}

pub(super) fn doc_line(leading: &str, node: Node<'_>, source: &str) -> String {
    let comment = leading
        .lines()
        .map(clean_comment)
        .find(|line| !line.is_empty());
    let docstring = || {
        // A Python docstring is the first statement of the body.
        let body = node.child_by_field_name("body")?;
        let first = body.named_child(0)?;
        let string = (first.kind() == "expression_statement")
            .then(|| first.named_child(0))
            .flatten()
            .filter(|n| n.kind() == "string")?;
        text(string, source)
            .trim_matches(['"', '\'', 'r', 'b', 'f'])
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string)
    };
    clip(&comment.or_else(docstring).unwrap_or_default(), DOC_CHARS)
}

fn clean_comment(line: &str) -> String {
    let line = line.trim();
    if line.starts_with("#[") {
        return String::new();
    }
    let mut line = line
        .trim_start_matches(['/', '*', '!', '#'])
        .trim_end_matches("*/")
        .to_string();
    for tag in DOC_TAGS {
        line = line.replace(tag, "");
    }
    line.trim().to_string()
}

/// C# XML documentation tags that wrap the text of a comment.
const DOC_TAGS: [&str; 8] = [
    "<summary>",
    "</summary>",
    "<remarks>",
    "</remarks>",
    "<returns>",
    "</returns>",
    "<para>",
    "</para>",
];

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}…", text.chars().take(limit).collect::<String>())
}
