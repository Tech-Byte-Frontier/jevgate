//! Rust tests: `#[test]`-like attributes, `cfg(test)` items and modules
//! holding only tests, and a file compiled only for tests.
use super::child_text;
use tree_sitter::Node;

pub(super) fn rust_test_span(node: Node<'_>, source: &str) -> Option<(usize, usize)> {
    if !matches!(
        node.kind(),
        "function_item" | "function_signature_item" | "mod_item" | "impl_item"
    ) {
        return None;
    }
    let marked = preceding_attributes(node, source)
        .iter()
        .any(|text| attribute_marks_test(text))
        || has_inner_cfg_test(node, source)
        || mod_contains_only_tests(node, source);
    if !marked {
        return None;
    }
    Some((attribute_start(node), node.end_byte()))
}

fn mod_contains_only_tests(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "mod_item" {
        return false;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut saw_test = false;
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind().contains("comment")
            || matches!(
                child.kind(),
                "attribute_item" | "inner_attribute_item" | "use_declaration"
            )
        {
            continue;
        }
        if rust_test_span(child, source).is_some() {
            saw_test = true;
            continue;
        }
        return false;
    }
    saw_test
}

pub(super) fn whole_file_cfg_test(root: Node<'_>, source: &str) -> bool {
    let mut cursor = root.walk();
    root.named_children(&mut cursor).any(|child| {
        child.kind() == "inner_attribute_item" && cfg_is_test_only(child_text(child, source))
    })
}

fn has_inner_cfg_test(node: Node<'_>, source: &str) -> bool {
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    body.named_children(&mut cursor).any(|child| {
        child.kind() == "inner_attribute_item" && cfg_is_test_only(child_text(child, source))
    })
}

pub(crate) fn preceding_attributes<'a>(node: Node<'_>, source: &'a str) -> Vec<&'a str> {
    let mut texts = Vec::new();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() == "attribute_item" {
            texts.push(child_text(sibling, source));
        } else if !sibling.kind().contains("comment") {
            break;
        }
        previous = sibling.prev_named_sibling();
    }
    texts
}

fn attribute_start(node: Node<'_>) -> usize {
    let mut start = node.start_byte();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() == "attribute_item" {
            start = sibling.start_byte();
        } else if !sibling.kind().contains("comment") {
            break;
        }
        previous = sibling.prev_named_sibling();
    }
    start
}

pub(crate) fn attribute_marks_test(text: &str) -> bool {
    cfg_is_test_only(text) || attribute_path(text).is_some_and(is_test_attribute)
}

fn attribute_path(text: &str) -> Option<&str> {
    let start = text.find('[')? + 1;
    let end = text.rfind(']')?;
    let body = text.get(start..end)?.trim();
    let path = body.split(['(', '=']).next()?.trim();
    (!path.is_empty()).then_some(path)
}

fn is_test_attribute(path: &str) -> bool {
    path == "test"
        || path.ends_with("::test")
        || path == "rstest"
        || path.ends_with("::rstest")
        || path == "test_case"
        || path.ends_with("::test_case")
}

/// `cfg(test)` and `all(..., test, ...)` compile only for tests.
/// `not(test)` and `any(test, ...)` can still be production code, so they stay.
fn cfg_is_test_only(text: &str) -> bool {
    let cleaned = strip_strings(text);
    let mut rest = cleaned.as_str();
    while let Some(index) = rest.find("cfg") {
        let boundary = index == 0
            || !rest.as_bytes()[index - 1].is_ascii_alphanumeric()
                && rest.as_bytes()[index - 1] != b'_';
        let after = rest[index + 3..].trim_start();
        if boundary
            && let Some(body) = after.strip_prefix('(')
            && let Some(end) = matching_paren(body)
            && predicate_is_test_only(&body[..end])
        {
            return true;
        }
        rest = &rest[index + 3..];
    }
    false
}

fn predicate_is_test_only(expr: &str) -> bool {
    let expr = expr.trim();
    if expr == "test" {
        return true;
    }
    strip_call(expr, "all").is_some_and(|inner| {
        split_top_level(inner)
            .iter()
            .any(|part| predicate_is_test_only(part))
    })
}

fn strip_call<'a>(expr: &'a str, name: &str) -> Option<&'a str> {
    let rest = expr.trim().strip_prefix(name)?.trim_start();
    let rest = rest.strip_prefix('(')?;
    let end = matching_paren(rest)?;
    rest[end..].trim().is_empty().then_some(rest[..end].trim())
}

fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                let part = text[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn matching_paren(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(index),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn strip_strings(text: &str) -> String {
    let mut cleaned = String::new();
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '"' || character == '\'' {
            let quote = character;
            while let Some(next) = chars.next() {
                if next == '\\' {
                    chars.next();
                    continue;
                }
                if next == quote {
                    break;
                }
            }
            cleaned.push(' ');
        } else {
            cleaned.push(character);
        }
    }
    cleaned
}
