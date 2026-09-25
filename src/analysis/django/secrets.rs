//! Secrets in Django code: the names that mark a value as one, and the
//! literals assigned to them, shown redacted.
use crate::analysis::text;
use std::ops::Range;
use tree_sitter::Node;

/// Parts of a name that mark its value as a secret.
const SECRET_PARTS: [&str; 6] = [
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "TOKEN",
    "PRIVATE",
    "CREDENTIAL",
];

/// Whether a setting or dictionary key names a secret, such as `SECRET_KEY`,
/// `'PASSWORD'` or `AWS_SECRET_ACCESS_KEY`; `…_KEY` counts too.
pub fn secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_PARTS.iter().any(|p| upper.contains(p)) || upper.ends_with("_KEY") || upper == "KEY"
}

/// Byte ranges of the contents of string literals assigned to secret names
/// in these statements: `SECRET_KEY = '…'` and `'PASSWORD': '…'` in a
/// dictionary. They are shown redacted: the literal is evidence that the
/// secret is fixed in code; its value is never uploaded.
pub fn secret_literals(node: Node<'_>, source: &str, found: &mut Vec<Range<usize>>) {
    let value = match node.kind() {
        "assignment" => node
            .child_by_field_name("left")
            .filter(|l| l.kind() == "identifier" && secret_name(text(*l, source)))
            .and_then(|_| node.child_by_field_name("right")),
        "pair" => node
            .child_by_field_name("key")
            .filter(|k| k.kind() == "string" && secret_name(string_content(*k, source)))
            .and_then(|_| node.child_by_field_name("value")),
        "keyword_argument" => node
            .child_by_field_name("name")
            .filter(|n| secret_name(text(*n, source)))
            .and_then(|_| node.child_by_field_name("value")),
        _ => None,
    };
    if let Some(range) = value
        .and_then(literal_content)
        .filter(|r| !dotted_path(&source[r.clone()]))
    {
        found.push(range);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        secret_literals(child, source, found);
    }
}

/// A dotted Python path such as `app.auth.services.get_secret_key`: a
/// setting that names the function or class holding a secret, not a secret.
fn dotted_path(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            part.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && part.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
}

/// The text between the quotes of a string literal without interpolation.
fn string_content<'s>(node: Node<'_>, source: &'s str) -> &'s str {
    literal_content(node).map_or("", |r| &source[r])
}

/// The byte range between the quotes of a plain, non-empty string literal.
pub(super) fn literal_content(node: Node<'_>) -> Option<Range<usize>> {
    if node.kind() != "string" {
        return None;
    }
    let mut cursor = node.walk();
    let parts: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    if parts.iter().any(|p| p.kind() == "interpolation") {
        return None;
    }
    let content = parts.iter().find(|p| p.kind() == "string_content")?;
    Some(content.byte_range()).filter(|r| !r.is_empty())
}

/// `source[range]` with each redacted literal inside it replaced by a note
/// of its length, so a secret's presence shows but its value does not.
pub fn redacted(source: &str, range: Range<usize>, redactions: &[Range<usize>]) -> String {
    let mut shown = String::new();
    let mut at = range.start;
    for secret in redactions
        .iter()
        .filter(|r| range.start <= r.start && r.end <= range.end)
    {
        if secret.start < at {
            continue;
        }
        shown.push_str(&source[at..secret.start]);
        let length = source[secret.clone()].chars().count();
        shown.push_str(&format!("<redacted {length}-character literal>"));
        at = secret.end;
    }
    shown.push_str(&source[at..range.end]);
    shown
}
