//! The program's own error classes, sent as evidence with each handler.
use super::{ErrorClasses, Scope};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Error classes shown with a handler question, whole, up to this many bytes.
const ERROR_CLASS_BYTES: usize = 3000;

/// Classes named `…Error` or `…Exception` in selected application files and
/// context, whole, in path order, while they fit.
pub(super) fn error_classes(scope: &Scope<'_>, hashes: &BTreeMap<PathBuf, String>) -> ErrorClasses {
    let mut classes = ErrorClasses {
        text: String::new(),
        sources: Vec::new(),
    };
    let selected = scope
        .owners
        .iter()
        .filter(|o| scope.views[o].application)
        .map(|&o| {
            let input = &scope.inputs[o];
            (
                input.result.path.as_path(),
                input.source.as_deref().unwrap_or(""),
            )
        });
    let context = scope
        .context
        .iter()
        .map(|(path, source, _)| (path.as_path(), *source));
    let mut all: Vec<_> = selected.chain(context).collect();
    all.sort_by_key(|(path, _)| *path);
    for (path, source) in all {
        for text in error_class_texts(path, source) {
            if classes.text.len() + text.len() > ERROR_CLASS_BYTES {
                continue;
            }
            if !classes.text.is_empty() {
                classes.text.push_str("\n\n");
            }
            classes.text.push_str(text);
            if let Some(hash) = hashes.get(path)
                && !classes.sources.iter().any(|(p, _)| p == path)
            {
                classes.sources.push((path.to_path_buf(), hash.clone()));
            }
        }
    }
    classes
}

/// The whole text of each class whose name ends in `Error` or `Exception`:
/// through its closing brace, or its indented body in Python. In Rust, each
/// such enum or struct with the attributes above it, which hold the
/// messages `thiserror` writes.
fn error_class_texts<'a>(path: &Path, source: &'a str) -> Vec<&'a str> {
    let python = path.extension().is_some_and(|e| e == "py");
    let rust = path.extension().is_some_and(|e| e == "rs");
    let csharp = path.extension().is_some_and(|e| e == "cs");
    let mut found = Vec::new();
    let mut offset = 0;
    let mut attributes: Option<usize> = None;
    for line in source.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        if rust && line.trim_start().starts_with("#[") {
            attributes.get_or_insert(line_start);
            continue;
        }
        let start = if rust {
            attributes.take().unwrap_or(line_start)
        } else {
            line_start
        };
        let error = declared_class(line, rust, csharp)
            .is_some_and(|name| name.ends_with("Error") || name.ends_with("Exception"));
        if !error {
            continue;
        }
        let end = if python {
            Some(indented_end(&source[start..], line.len()))
        } else if (rust || csharp)
            && let Some(end) = declaration_end(&source[line_start..])
        {
            Some(line_start - start + end)
        } else {
            braced_end(&source[start..])
        };
        if let Some(end) = end {
            found.push(source[start..start + end].trim_end());
        }
    }
    found
}

/// The class a line declares: a JavaScript, TypeScript, Python or C# class,
/// or a Rust enum or struct.
fn declared_class(line: &str, rust: bool, csharp: bool) -> Option<String> {
    let rest = if csharp {
        let mut rest = line.trim_start();
        while let Some(after) = CSHARP_MODIFIERS
            .iter()
            .find_map(|modifier| rest.strip_prefix(modifier))
        {
            rest = after;
        }
        rest.strip_prefix("class ")
            .or_else(|| rest.strip_prefix("record "))?
    } else if rust {
        let visible = line.trim_start().trim_start_matches("pub(crate) ");
        let visible = visible.trim_start_matches("pub ");
        visible
            .strip_prefix("enum ")
            .or_else(|| visible.strip_prefix("struct "))?
    } else {
        line.trim_start_matches("export ")
            .trim_start_matches("default ")
            .trim_start_matches("abstract ")
            .strip_prefix("class ")?
    };
    Some(
        rest.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect(),
    )
}

/// Modifiers a C# class declaration can start with.
const CSHARP_MODIFIERS: [&str; 7] = [
    "public ",
    "internal ",
    "private ",
    "protected ",
    "sealed ",
    "abstract ",
    "partial ",
];

/// The end of a Python class: its first line and the indented lines after it.
fn indented_end(text: &str, first: usize) -> usize {
    let mut end = first;
    for next in text[first..].split_inclusive('\n') {
        if !next.trim().is_empty() && !next.starts_with([' ', '\t']) {
            break;
        }
        end += next.len();
    }
    end
}

/// The end of a declaration through the brace that closes its first `{`.
fn braced_end(text: &str) -> Option<usize> {
    let open = text.find('{')?;
    let mut depth = 0usize;
    text[open..].char_indices().find_map(|(i, c)| {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        (depth == 0).then_some(open + i + 1)
    })
}

/// Where a Rust unit or tuple struct declaration, or a C# class with a
/// primary constructor and no body, ends: at its `;`, when that comes before
/// any `{` outside parentheses. A C# base call can interpolate
/// (`: Exception($"{name} was not found");`).
fn declaration_end(declaration: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in declaration.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => return Some(i + 1),
            '{' if depth == 0 => return None,
            _ => {}
        }
    }
    None
}
