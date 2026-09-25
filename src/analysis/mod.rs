//! Free local analysis over the selected scope: units, member groups, Type-2
//! clone candidates and a test map. Parsers supply evidence and locations;
//! every judgment about meaning is left to Jev.
pub mod blocks;
pub mod clones;
pub mod django;
pub mod errors;
pub mod groups;
pub mod imports;
pub mod literals;
pub mod nesting;
pub(crate) mod php;
pub mod routes;
pub mod ruby;
pub mod sites;
pub mod sql;
mod summary;
pub mod test_map;
pub mod units;
pub mod workflow;

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
            // Java: `new ArrayList<>()` and `new Outer.Inner()`.
            "generic_type" => node.named_child(0)?,
            "scoped_type_identifier" => {
                node.named_child(node.named_child_count().checked_sub(1)? as u32)?
            }
            "scoped_identifier" => node.child_by_field_name("name")?,
            "field_expression" => node.child_by_field_name("field")?,
            "member_expression" => node.child_by_field_name("property")?,
            "selector_expression" => node.child_by_field_name("field")?,
            "scope_resolution" => node.child_by_field_name("name")?,
            "attribute" => node.child_by_field_name("attribute")?,
            // C#: `_repository.ListAsync`, `Get<T>`, `System.IO.File` and `order?.Total()`.
            "member_access_expression" | "member_binding_expression" | "qualified_name" => {
                node.child_by_field_name("name")?
            }
            "generic_name" => node.named_child(0)?,
            "conditional_access_expression" => {
                let mut cursor = node.walk();
                node.named_children(&mut cursor)
                    .find(|c| c.kind() == "member_binding_expression")?
            }
            "identifier"
            | "field_identifier"
            | "property_identifier"
            | "type_identifier"
            | "constant" => {
                return Some(text(node, source).to_string());
            }
            _ => return None,
        };
    }
}

/// The name a call is recorded under: the last segment of its `function`, or
/// the `method` a Ruby call names. Ruby's `Billing::Invoice.new(…)` builds an
/// `Invoice`, like JavaScript's `new Invoice(…)`, rather than calling some `new`.
pub(crate) fn call_name(call: Node<'_>, source: &str) -> Option<String> {
    if let Some(class) = call
        .child_by_field_name("receiver")
        .filter(|r| matches!(r.kind(), "constant" | "scope_resolution"))
        .filter(|_| ruby::method(call, source) == "new")
    {
        return callee_name(class, source);
    }
    call.child_by_field_name("function")
        .or_else(|| call.child_by_field_name("method"))
        .and_then(|callee| callee_name(callee, source))
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

/// A fast in-process hash for comparing token sequences; never persisted.
pub(crate) fn fast_hash<T: std::hash::Hash + ?Sized>(value: &T) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn is_comment(node: Node<'_>) -> bool {
    node.kind().contains("comment")
}
