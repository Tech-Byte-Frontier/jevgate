//! Free local analysis over the selected scope: units, member groups, Type-2
//! clone candidates and a test map. Parsers supply evidence and locations;
//! every judgment about meaning is left to Jev.
pub mod clones;
pub mod groups;
pub mod test_map;
pub mod units;

use tree_sitter::Node;

pub(crate) fn line_of(source: &str, byte: usize) -> usize {
    source[..byte.min(source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

pub(crate) fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

/// The last name segment of a callee such as `a.b::<T>`, `Self::run` or `obj.method`.
pub(crate) fn callee_name(node: Node<'_>, source: &str) -> Option<String> {
    let mut node = node;
    loop {
        node = match node.kind() {
            "generic_function" => node.child_by_field_name("function")?,
            "scoped_identifier" => node.child_by_field_name("name")?,
            "field_expression" => node.child_by_field_name("field")?,
            "member_expression" => node.child_by_field_name("property")?,
            "attribute" => node.child_by_field_name("attribute")?,
            "identifier" | "field_identifier" | "property_identifier" | "type_identifier" => {
                return Some(text(node, source).to_string());
            }
            _ => return None,
        };
    }
}

/// Names called inside a Rust macro's token tree, such as `total` in
/// `assert_eq!(total(&values), 3)`. Tree-sitter leaves macro arguments unparsed.
pub(crate) fn macro_calls(
    node: Node<'_>,
    source: &str,
    calls: &mut std::collections::BTreeSet<String>,
) {
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
    for pair in children.windows(2) {
        if pair[0].kind() == "identifier"
            && pair[1].kind() == "token_tree"
            && text(pair[1], source).starts_with('(')
        {
            calls.insert(text(pair[0], source).to_string());
        }
    }
    for child in children {
        if child.kind() == "token_tree" {
            macro_calls(child, source, calls);
        }
    }
}

pub(crate) fn is_comment(node: Node<'_>) -> bool {
    node.kind().contains("comment")
}
