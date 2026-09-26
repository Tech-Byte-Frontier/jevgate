//! Splitting PostgreSQL text into statements: comments, quoted strings and
//! identifiers, and dollar-quoted bodies never end a statement.
use super::Statement;

/// Statements in order, without leading comments; blank statements are dropped.
pub fn statements(text: &str) -> Vec<Statement> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let (mut i, mut start) = (0, 0);
    while i < bytes.len() {
        if text[i..].starts_with("--") {
            i = text[i..].find('\n').map_or(bytes.len(), |j| i + j);
        } else if text[i..].starts_with("/*") {
            i = text[i + 2..]
                .find("*/")
                .map_or(bytes.len(), |j| i + 2 + j + 2);
        } else if bytes[i] == b'\'' || bytes[i] == b'"' {
            i = quoted_end(bytes, i);
        } else if let Some(tag) = dollar_tag(&text[i..]) {
            let body = i + tag.len();
            i = text[body..]
                .find(tag)
                .map_or(bytes.len(), |j| body + j + tag.len());
        } else if bytes[i] == b';' {
            spans.push(start..i + 1);
            i += 1;
            start = i;
        } else {
            i += text[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    spans.push(start..bytes.len());
    spans
        .into_iter()
        .filter_map(|span| {
            let offset = span.start + leading_comments(&text[span.clone()]);
            let source = text[offset..span.end].trim();
            (!source.is_empty() && source != ";").then(|| Statement {
                start_line: crate::analysis::line_of(text, offset),
                end_line: crate::analysis::line_of(text, span.end.saturating_sub(1).max(offset)),
                source: source.to_string(),
            })
        })
        .collect()
}

/// The index after a quoted string or identifier; doubled quotes escape.
pub(super) fn quoted_end(bytes: &[u8], open: usize) -> usize {
    let quote = bytes[open];
    let mut j = open + 1;
    while j < bytes.len() {
        if bytes[j] == quote {
            if bytes.get(j + 1) == Some(&quote) {
                j += 2;
                continue;
            }
            return j + 1;
        }
        j += 1;
    }
    bytes.len()
}

/// `$tag$` or `$$` opening a dollar-quoted body.
fn dollar_tag(rest: &str) -> Option<&str> {
    let tail = rest.strip_prefix('$')?;
    let end = tail.find('$')?;
    tail[..end]
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        .then(|| &rest[..end + 2])
}

/// Bytes of whitespace and comments before a statement's first token.
fn leading_comments(text: &str) -> usize {
    let mut i = 0;
    loop {
        let rest = &text[i..];
        let trimmed = rest.trim_start();
        i += rest.len() - trimmed.len();
        if trimmed.starts_with("--") {
            i += trimmed.find('\n').map_or(trimmed.len(), |j| j + 1);
        } else if trimmed.starts_with("/*") {
            i += trimmed.find("*/").map_or(trimmed.len(), |j| j + 2);
        } else {
            return i;
        }
    }
}
