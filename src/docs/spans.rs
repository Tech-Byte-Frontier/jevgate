//! Inline code spans and fenced code blocks, which the references rule
//! reads paths from: in Markdown, and in reStructuredText and MyST, whose
//! interpreted-text roles say whether a span names a file.

use super::markdown::{Fenced, Fences};

/// Interpreted-text roles whose target is a file: reStructuredText's
/// `:file:` and `:download:`, and the same roles written the MyST way.
const PATH_ROLES: &[&str] = &["file", "download", "doc"];

/// The lines outside fenced code blocks, and the fenced lines, of `text`.
pub(super) fn split_fences(text: &str) -> (Vec<&str>, Vec<&str>) {
    let (mut prose, mut code) = (Vec::new(), Vec::new());
    let mut fences = Fences::default();
    for line in text.lines() {
        match fences.line(line) {
            Fenced::Inside => code.push(line),
            Fenced::Outside => prose.push(line),
            Fenced::Opens(_) | Fenced::Closes => {}
        }
    }
    (prose, code)
}

/// Inline code spans of one line, each with the text before it.
pub(super) fn line_spans(line: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = line;
    let mut offset = 0;
    while let Some(open) = rest.find('`') {
        let run = rest[open..].chars().take_while(|&c| c == '`').count();
        let body = &rest[open + run..];
        let fence = "`".repeat(run);
        // The closing run has the same length as the opening one.
        let close = body.match_indices(&fence).find(|(i, _)| {
            !body[i + run..].starts_with('`') && (*i == 0 || !body[..*i].ends_with('`'))
        });
        let Some((close, _)) = close else { break };
        out.push((&line[..offset + open], &body[..close]));
        let consumed = open + run + close + run;
        offset += consumed;
        rest = &rest[consumed..];
    }
    out
}

/// The role an interpreted-text span is written with, such as `attr` in
/// `:py:attr:` or `{file}`, from the text before the span.
fn role(before: &str) -> Option<&str> {
    if let Some(inner) = before.strip_suffix('}') {
        let start = inner.rfind('{')?;
        return Some(&inner[start + 1..]);
    }
    let inner = before.strip_suffix(':')?;
    let start = inner
        .char_indices()
        .rfind(|&(_, c)| !(c.is_ascii_alphanumeric() || c == ':' || c == '-' || c == '_'))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let name = inner[start..].strip_prefix(':')?;
    (!name.is_empty()).then(|| name.rsplit(':').next().unwrap_or(name))
}

/// Whether a code span written after `before` can name a file: one without
/// a role, or with a file role. A span written with a role names what the
/// role says: `:attr:` and `:class:` name code, only `:file:` and its kind
/// name files.
pub(super) fn file_span(before: &str) -> bool {
    role(before).is_none_or(|r| PATH_ROLES.contains(&r))
}

/// Inline code spans without whitespace, outside code blocks, that can name
/// a file.
pub(super) fn code_spans(text: &str) -> Vec<String> {
    split_fences(text)
        .0
        .into_iter()
        .flat_map(line_spans)
        .filter(|(before, _)| file_span(before))
        .map(|(_, span)| span.trim())
        .filter(|span| !span.is_empty() && !span.contains(char::is_whitespace))
        .map(str::to_string)
        .collect()
}
