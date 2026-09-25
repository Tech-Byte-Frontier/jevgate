//! Comments of one file, each with the code it is about, for the comments
//! rule. Consecutive comments on their own lines are one comment. License
//! headers, tool directives, type annotations and decorations are left out:
//! a reader cannot do without them, or they hold no prose to judge.
use super::{is_comment, line_of, units::Unit};
use anyhow::Result;
use std::{collections::BTreeSet, ops::Range, path::Path};
use tree_sitter::Node;

/// Lines of code shown after a comment above code, at most.
const CODE_LINES: usize = 8;
/// A documented declaration longer than this is shown by its signature.
const DECLARATION_LINES: usize = 40;
/// Signatures shown for a file's own documentation, at most.
const FILE_SIGNATURES: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Documentation directly above a declaration, or a Python docstring.
    Declaration,
    /// Documentation of the whole file, before any of its code.
    File,
    /// On its own lines, above or after the code it is about.
    Above,
    /// At the end of a line of code.
    Trailing,
}

#[derive(Clone, Debug)]
pub struct Comment {
    pub span: Range<usize>,
    pub line: usize,
    pub end_line: usize,
    pub text: String,
    pub placement: Placement,
    /// The code the comment is about: the declaration it documents (its
    /// signature when long), the lines below it, the line it ends, or for a
    /// file's documentation the signatures of its top-level definitions.
    pub code: String,
    /// The innermost unit it documents or sits in, as an index into the units.
    pub unit: Option<usize>,
    /// Most of its lines read like statements, so it may be code turned off.
    pub code_like: bool,
    /// Words of its prose, without comment markers.
    pub words: usize,
    /// The name of the definition a docstring opens when no unit holds it,
    /// such as a class of fields only.
    pub definition: Option<String>,
}

/// The eligible comments of a file in a supported language, in order.
pub fn comments(path: &Path, source: &str, units: &[Unit]) -> Result<Vec<Comment>> {
    let Some(tree) = crate::syntax::parse(path, source)? else {
        return Ok(Vec::new());
    };
    let mut raw = Vec::new();
    collect(tree.root_node(), source, &mut raw);
    raw.sort_by_key(|r: &Raw| r.span.start);
    let lines: Vec<&str> = source.split('\n').collect();
    let blocks = merge(raw, source);
    // Lines where a comment on its own line starts: code shown below a
    // comment stops there.
    let starts: BTreeSet<usize> = blocks
        .iter()
        .filter(|b| own_line(source, b.span.start))
        .map(|b| line_of(source, b.span.start))
        .collect();
    let first_code = first_code_byte(tree.root_node(), &blocks);
    let mut found = Vec::new();
    for block in blocks {
        let text = &source[block.span.clone()];
        let line = line_of(source, block.span.start);
        let text = &without_version_notes(text);
        if !eligible(text, line) {
            continue;
        }
        let end_line = last_line(source, &block.span);
        let comment = Comment {
            span: block.span.clone(),
            line,
            end_line,
            text: text.to_string(),
            placement: Placement::Above,
            code: String::new(),
            unit: None,
            code_like: !block.docstring && code_like(text),
            words: prose(text).split_whitespace().count(),
            definition: None,
        };
        found.push(place(
            comment, &block, source, &lines, units, &starts, first_code,
        ));
    }
    Ok(found)
}

struct Raw {
    span: Range<usize>,
    docstring: bool,
    /// The function or class a docstring opens, and its name.
    definition: Option<(Range<usize>, String)>,
}

/// Comment nodes, without the parts a comment node holds (a Rust doc
/// comment's marker and text are comments too), and Python docstrings.
fn collect(node: Node<'_>, source: &str, found: &mut Vec<Raw>) {
    if is_comment(node) {
        found.push(Raw {
            span: node.byte_range(),
            docstring: false,
            definition: None,
        });
        return;
    }
    if let Some(string) = docstring(node) {
        // The body's definition, with its decorators.
        let definition = node.parent().and_then(|b| b.parent()).map(|d| {
            let range = d
                .parent()
                .filter(|p| p.kind() == "decorated_definition")
                .unwrap_or(d)
                .byte_range();
            let name = d
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or_default()
                .to_string();
            (range, name)
        });
        found.push(Raw {
            span: string.byte_range(),
            docstring: true,
            definition: definition.filter(|_| node.parent().is_some_and(|b| b.kind() == "block")),
        });
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, source, found);
    }
}

/// The string of an expression statement that opens a Python module,
/// function or class body.
pub(crate) fn docstring(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() != "expression_statement" || node.named_child_count() != 1 {
        return None;
    }
    let string = node.named_child(0).filter(|n| n.kind() == "string")?;
    let parent = node.parent()?;
    let opens = match parent.kind() {
        "module" => true,
        "block" => parent
            .parent()
            .is_some_and(|p| matches!(p.kind(), "function_definition" | "class_definition")),
        _ => false,
    };
    let mut cursor = parent.walk();
    let first = parent
        .named_children(&mut cursor)
        .find(|c| !is_comment(*c))
        .is_some_and(|c| c.id() == node.id());
    (opens && first).then_some(string)
}

struct Block {
    span: Range<usize>,
    docstring: bool,
    definition: Option<(Range<usize>, String)>,
}

/// Consecutive line comments on their own lines, one directly below the
/// other and written with the same marker, are one comment; block comments
/// stand alone.
fn merge(raw: Vec<Raw>, source: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    for comment in raw {
        if let Some(last) = blocks.last_mut()
            && !last.docstring
            && !comment.docstring
            && own_line(source, last.span.start)
            && own_line(source, comment.span.start)
            && line_marker(&source[last.span.clone()]).is_some()
            && line_marker(&source[last.span.clone()]) == line_marker(&source[comment.span.clone()])
            && last.span.end <= comment.span.start
            && source[last.span.end..comment.span.start].trim().is_empty()
            && line_of(source, comment.span.start) == last_line(source, &last.span) + 1
        {
            last.span.end = comment.span.end;
            continue;
        }
        blocks.push(Block {
            span: comment.span,
            docstring: comment.docstring,
            definition: comment.definition,
        });
    }
    blocks
}

/// The line a span ends on; a line comment may hold its own newline.
fn last_line(source: &str, span: &Range<usize>) -> usize {
    line_of(source, span.start)
        + source[span.clone()]
            .trim_end_matches(['\n', '\r'])
            .matches('\n')
            .count()
}

/// The marker of a line comment: `//`, `///`, `//!` or `#`; none for a block.
fn line_marker(text: &str) -> Option<&str> {
    ["///", "//!", "//", "#"]
        .into_iter()
        .find(|m| text.starts_with(m))
}

fn line_start(source: &str, byte: usize) -> usize {
    source[..byte].rfind('\n').map_or(0, |i| i + 1)
}

fn own_line(source: &str, byte: usize) -> bool {
    source[line_start(source, byte)..byte].trim().is_empty()
}

/// The first byte of code that is not a comment, a docstring, a shebang or
/// an import: a file's own documentation comes before it.
fn first_code_byte(root: Node<'_>, blocks: &[Block]) -> usize {
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        // PHP's opening tag starts the file; the code comes after it.
        .filter(|c| !is_comment(*c) && c.kind() != "php_tag")
        .filter(|c| {
            !blocks
                .iter()
                .any(|b| b.docstring && c.byte_range().contains(&b.span.start))
        })
        .map(|c| c.start_byte())
        .next()
        .unwrap_or(usize::MAX)
}

/// A section banner framed by rules of dashes, as Laravel's skeleton heads
/// each configuration section and route file: `|------|`, a title, then an
/// explanation. It documents the section below it, like a docstring.
pub fn banner(text: &str) -> bool {
    text.lines().any(|line| {
        let rule = line.trim().trim_start_matches(['/', '*', '#', ' ']);
        rule.strip_prefix('|')
            .is_some_and(|rest| rest.len() >= 10 && rest.chars().all(|c| c == '-'))
    })
}

/// Words a reader reads: the text without comment markers.
pub fn prose(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.trim()
                .trim_start_matches(|c: char| "/*#!\"'=".contains(c))
                .trim_end_matches("*/")
                .trim_end_matches(['"', '\''])
                .trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Prefixes of comments that instruct a tool rather than a reader.
const DIRECTIVES: &[&str] = &[
    "eslint",
    "prettier-ignore",
    "@ts-",
    "tslint:",
    "istanbul ",
    "c8 ",
    "v8 ignore",
    "noqa",
    "type:",
    "pylint:",
    "pyright:",
    "mypy:",
    "pyre-",
    "pytype:",
    "ruff:",
    "fmt:",
    "isort:",
    "yapf:",
    "nolint",
    "go:",
    "+build",
    "+kubebuilder",
    "rubocop:",
    "frozen_string_literal",
    "encoding:",
    "coding:",
    "-*-",
    "region",
    "endregion",
    "#region",
    "#endregion",
    "phpcs",
    "@phpstan",
    "@psalm",
    "nosonar",
    "language=",
    "@formatter",
    "checkstyle",
    "@jsx",
    "<reference",
    "webpackchunkname",
    "@vite-ignore",
    "biome-ignore",
    "deno-lint",
    "deno-fmt",
    "dprint-ignore",
    "jscpd:",
    "pragma",
    "@flow",
    "@refresh",
    "@license",
    "@preserve",
    "sourcemappingurl",
    "cspell:",
    "spell-checker:",
    "codespell:",
    "markdownlint",
    "stylelint",
    "jshint",
    "jslint",
    "global ",
    "@vitest-environment",
    "@jest-environment",
    "@type ",
    "@type{",
    "@satisfies",
    "@typedef",
    "@import",
    "@generated",
    "rustfmt::",
    "clippy::",
];

/// Sphinx directives that record the release a behavior appeared or changed
/// in, with their indented bodies: documentation tools expect them, and
/// flask's `.. versionchanged:: 2.2` read as narrating an edit.
const VERSION_NOTES: [&str; 3] = [
    ".. versionadded::",
    ".. versionchanged::",
    ".. deprecated::",
];

/// A comment's text without its Sphinx version notes.
fn without_version_notes(text: &str) -> std::borrow::Cow<'_, str> {
    if !VERSION_NOTES.iter().any(|note| text.contains(note)) {
        return text.into();
    }
    let indent = |line: &str| line.len() - line.trim_start().len();
    let mut kept = Vec::new();
    let mut inside: Option<usize> = None;
    for line in text.split('\n') {
        if let Some(depth) = inside {
            if line.trim().is_empty() || indent(line) > depth {
                continue;
            }
            inside = None;
        }
        if VERSION_NOTES
            .iter()
            .any(|note| line.trim_start().starts_with(note))
        {
            inside = Some(indent(line));
            continue;
        }
        kept.push(line);
    }
    kept.join("\n").into()
}

/// A comment with prose a reader could do without: not a directive, a
/// license or generated-code header, a shebang or a decoration.
fn eligible(text: &str, line: usize) -> bool {
    if text.starts_with("#!") {
        return false;
    }
    let words = prose(text).to_lowercase();
    if !words.chars().any(char::is_alphabetic) {
        return false;
    }
    if DIRECTIVES.iter().any(|d| words.starts_with(d)) {
        return false;
    }
    let license = words.contains("copyright")
        || words.contains("spdx-license-identifier")
        || (line <= 3 && words.contains("license"));
    let generated = [
        "auto-generated",
        "autogenerated",
        "automatically generated",
        "do not edit",
    ]
    .iter()
    .any(|m| words.contains(m));
    !license && !generated
}

/// Starts of lines that are statements rather than prose.
const STATEMENTS: &[&str] = &[
    "return ", "if (", "if ", "for (", "for ", "while ", "const ", "let ", "var ", "import ",
    "from ", "def ", "fn ", "pub ", "await ", "console.", "print(", "self.", "this.",
];

/// Whether most of a comment's lines read like code: they end as a
/// statement or block does, or start as one.
fn code_like(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(|c: char| "/*#".contains(c))
                .trim_end_matches("*/")
                .trim()
        })
        .filter(|l| !l.is_empty())
        .collect();
    let code = lines
        .iter()
        .filter(|l| {
            l.ends_with([';', '{', '}', ')', ',']) || STATEMENTS.iter().any(|s| l.starts_with(s))
        })
        .count();
    !lines.is_empty() && code * 2 >= lines.len()
}

/// Where a comment sits and the code it is about.
fn place(
    mut comment: Comment,
    block: &Block,
    source: &str,
    lines: &[&str],
    units: &[Unit],
    starts: &BTreeSet<usize>,
    first_code: usize,
) -> Comment {
    let span = &block.span;
    if block.docstring {
        place_docstring(&mut comment, block, source, lines, units);
    } else if !own_line(source, span.start) {
        comment.placement = Placement::Trailing;
        comment.code = source[line_start(source, span.start)..span.start]
            .trim()
            .to_string();
        comment.unit = enclosing(units, &comment);
    } else if let Some(index) = documented(units, lines, &comment) {
        let unit = &units[index];
        comment.placement = Placement::Declaration;
        comment.unit = Some(index);
        let definition = source_offset(lines, comment.end_line + 1);
        comment.code = declaration_code(&source[definition..unit.span.end], unit);
    } else if span.end <= first_code {
        comment.placement = Placement::File;
        comment.code = file_code(units);
    } else {
        comment.unit = enclosing(units, &comment);
        comment.code = nearby(lines, &comment, starts);
    }
    if comment.unit.is_none() && comment.definition.is_none() {
        comment.definition = member_of(units, &comment, lines);
    }
    comment
}

/// A docstring documents the innermost definition it opens, or the module.
fn place_docstring(
    comment: &mut Comment,
    block: &Block,
    source: &str,
    lines: &[&str],
    units: &[Unit],
) {
    let span = &block.span;
    let owner = innermost(units, |u| {
        u.span.contains(&span.start) && u.line < comment.line
    });
    comment.unit = owner;
    comment.placement = Placement::Declaration;
    if let Some(index) = owner {
        let unit = &units[index];
        let definition = line_start(source, source_offset(lines, unit.line));
        let shown = without_docstring(source, definition..unit.span.end, span);
        comment.code = declaration_code(&shown, unit);
    } else if let Some((definition, name)) = &block.definition {
        // A definition the parser keeps no unit for, such as a class of
        // fields only: shown whole, or by its first line when long.
        comment.definition = Some(name.clone()).filter(|n| !n.is_empty());
        let shown = without_docstring(source, definition.clone(), span);
        comment.code = if shown.lines().count() <= DECLARATION_LINES {
            shown.trim_end().to_string()
        } else {
            shown.lines().take(2).collect::<Vec<_>>().join("\n")
        };
    } else {
        comment.placement = Placement::File;
        comment.code = file_code(units);
    }
}

/// A definition's source without the lines of its docstring at `docstring`,
/// so the rest reads as code.
fn without_docstring(source: &str, definition: Range<usize>, docstring: &Range<usize>) -> String {
    let after = source[docstring.end..definition.end]
        .find('\n')
        .map_or(definition.end, |i| docstring.end + i + 1);
    format!(
        "{}{}",
        &source[definition.start..line_start(source, docstring.start)],
        &source[after..definition.end]
    )
}

/// The unit a comment directly above documents, with only attributes or
/// decorators between them. A declaration's own span may start after the
/// comment's line, as `function` does inside `export function`.
fn documented(units: &[Unit], lines: &[&str], comment: &Comment) -> Option<usize> {
    innermost(units, |u| {
        comment.end_line < u.line && (comment.end_line + 1..u.line).all(|l| attribute(lines[l - 1]))
    })
}

/// The byte offset where 1-based `line` starts.
fn source_offset(lines: &[&str], line: usize) -> usize {
    lines.iter().take(line - 1).map(|l| l.len() + 1).sum()
}

fn declaration_code(shown: &str, unit: &Unit) -> String {
    if shown.lines().count() <= DECLARATION_LINES {
        shown.trim_end().to_string()
    } else {
        unit.signature.clone()
    }
}

/// The signatures of the file's top-level definitions, one per line.
fn file_code(units: &[Unit]) -> String {
    units
        .iter()
        .filter(|u| u.owner.is_empty())
        .take(FILE_SIGNATURES)
        .map(|u| u.signature.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The unit with the latest start among those `holds` accepts: the innermost.
fn innermost(units: &[Unit], holds: impl Fn(&Unit) -> bool) -> Option<usize> {
    units
        .iter()
        .enumerate()
        .filter(|(_, u)| holds(u))
        .max_by_key(|(_, u)| u.span.start)
        .map(|(i, _)| i)
}

/// The innermost unit whose definition holds the comment.
/// The type whose body holds a comment among its members, such as a PHP
/// property's docblock: a type with methods has no unit of its own, so the
/// comment read as the file's top-level code. The owner of the members
/// around it, or of the member after an indented comment.
fn member_of(units: &[Unit], comment: &Comment, lines: &[&str]) -> Option<String> {
    let before = units
        .iter()
        .filter(|u| u.end_line < comment.line)
        .max_by_key(|u| u.end_line);
    let after = units
        .iter()
        .filter(|u| u.line > comment.end_line)
        .min_by_key(|u| u.line)?;
    let indented = lines
        .get(comment.line - 1)
        .is_some_and(|line| indent(line) > 0);
    let same = before.is_some_and(|u| u.owner == after.owner);
    (!after.owner.is_empty() && (same || indented && before.is_none_or(|u| u.owner.is_empty())))
        .then(|| after.owner.clone())
}

fn enclosing(units: &[Unit], comment: &Comment) -> Option<usize> {
    innermost(units, |u| {
        u.line <= comment.line
            && comment.span.end <= u.span.end
            && u.span.start <= comment.span.start
    })
}

/// An attribute, annotation or decorator line: `#[…]`, `@…` or `[…]`.
fn attribute(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("#[") || line.starts_with('@') || line.starts_with('[')
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The lines below a comment on its own lines, up to a blank line, another
/// comment or the end of its block; past one blank line when it labels what
/// follows. A comment that closes a block is shown with the lines above it.
fn nearby(lines: &[&str], comment: &Comment, starts: &BTreeSet<usize>) -> String {
    let own = indent(lines[comment.line - 1]);
    let mut shown = Vec::new();
    let mut index = comment.end_line;
    while index < lines.len() && lines[index].trim().is_empty() && index == comment.end_line {
        index += 1;
    }
    while index < lines.len() && shown.len() < CODE_LINES {
        let line = lines[index];
        if line.trim().is_empty() || starts.contains(&(index + 1)) || indent(line) < own {
            break;
        }
        shown.push(line);
        index += 1;
    }
    if shown.is_empty() {
        let mut index = comment.line - 1;
        while index > 0 && shown.len() < CODE_LINES {
            let line = lines[index - 1];
            if line.trim().is_empty() || starts.contains(&index) || indent(line) < own {
                break;
            }
            shown.insert(0, line);
            index -= 1;
        }
    }
    shown.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(name: &str, source: &str) -> Vec<Comment> {
        let path = Path::new(name);
        let units = crate::analysis::units::parse(path, source).unwrap().units;
        comments(path, source, &units).unwrap()
    }

    #[test]
    fn consecutive_line_comments_are_one_comment_with_the_code_below() {
        let source = "fn total(values: &[i32]) -> i32 {\n    let mut sum = 0;\n    // Add every value\n    // to the sum.\n    for value in values {\n        sum += value;\n    }\n\n    sum\n}\n";
        let comments = found("lib.rs", source);
        assert_eq!(comments.len(), 1);
        let comment = &comments[0];
        assert_eq!((comment.line, comment.end_line), (3, 4));
        assert_eq!(comment.placement, Placement::Above);
        assert_eq!(
            comment.code,
            "    for value in values {\n        sum += value;\n    }"
        );
        assert_eq!(comment.unit, Some(0));
    }

    #[test]
    fn documentation_is_shown_with_its_declaration_and_a_trailing_comment_with_its_line() {
        let source = "/// Returns the name.\n#[inline]\npub fn name(&self) -> &str {\n    &self.name // the stored name\n}\n";
        let comments = found("lib.rs", source);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].placement, Placement::Declaration);
        assert!(
            comments[0].code.starts_with("#[inline]\npub fn name"),
            "{}",
            comments[0].code
        );
        assert_eq!(comments[1].placement, Placement::Trailing);
        assert_eq!(comments[1].code, "&self.name");
    }

    #[test]
    fn a_label_separated_by_a_blank_line_is_shown_with_what_follows() {
        let source = "// ===== Helpers =====\n\nfn helper() -> i32 {\n    1\n}\n\nfn other() -> i32 {\n    2\n}\n";
        let comments = found("lib.rs", source);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].placement, Placement::File);
        let source = format!("fn first() {{}}\n\n{source}");
        let comments = found("lib.rs", &source);
        assert_eq!(comments[0].placement, Placement::Above);
        assert!(comments[0].code.starts_with("fn helper() -> i32 {"));
    }

    #[test]
    fn python_docstrings_document_their_function_or_module() {
        let source = "\"\"\"Helpers for orders.\"\"\"\n\ndef total(items):\n    \"\"\"Return the total of the items.\"\"\"\n    return sum(i.price for i in items)\n";
        let comments = found("orders.py", source);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].placement, Placement::File);
        assert_eq!(comments[0].code, "def total(items)");
        assert_eq!(comments[1].placement, Placement::Declaration);
        assert_eq!(
            comments[1].code,
            "def total(items):\n    return sum(i.price for i in items)"
        );
    }

    #[test]
    fn a_decorated_class_docstring_documents_its_class() {
        let source = "import dataclasses\n\n@dataclasses.dataclass\nclass Result:\n    \"\"\"One result.\"\"\"\n\n    value: int\n";
        let comments = found("result.py", source);
        assert_eq!(comments[0].placement, Placement::Declaration);
        assert!(comments[0].unit.is_some());
        let source = "import dataclasses\n\n@dataclasses.dataclass(frozen=True)\nclass Result:\n    \"\"\"One result.\"\"\"\n\n    value: int\n";
        let comments = super::comments(Path::new("result.py"), source, &[]).unwrap();
        assert_eq!(comments[0].placement, Placement::Declaration);
        assert_eq!(comments[0].definition.as_deref(), Some("Result"));
        assert!(
            comments[0]
                .code
                .starts_with("@dataclasses.dataclass(frozen=True)\nclass Result:")
        );
    }

    #[test]
    fn directives_licenses_shebangs_and_decorations_are_left_out() {
        let source = "#!/usr/bin/env node\n// Copyright 2024 Example Inc.\n/* eslint-disable no-console */\nconst a = 1; // @ts-ignore\n// ---------------\n/** @type {import('next').NextConfig} */\nconst config = {};\n// Retry once: the first request after a deploy is often refused.\nfetchAgain();\n";
        let comments = found("app.js", source);
        let texts: Vec<&str> = comments.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            ["// Retry once: the first request after a deploy is often refused."]
        );
    }

    #[test]
    fn documentation_of_an_exported_function_documents_it() {
        let source = "/** Convert a name to a key. */\nexport function toKey(name: string): string {\n  return name.toLowerCase()\n}\n";
        let comments = found("keys.ts", source);
        assert_eq!(comments[0].placement, Placement::Declaration);
        assert!(comments[0].code.starts_with("export function toKey"));
        let source = format!("const a = 1\n// Keys\nconst b = 2\n\n{source}");
        assert_eq!(found("keys.ts", &source)[0].placement, Placement::Above);
    }

    #[test]
    fn only_comments_whose_lines_read_like_statements_may_be_code_turned_off() {
        assert!(code_like("// const total = sum(values);\n// return total;"));
        assert!(code_like("# print(result)"));
        assert!(!code_like(
            "// Retry once: the first request is often refused."
        ));
        assert!(!code_like("// Mutations"));
    }

    #[test]
    fn comments_among_a_type_s_members_belong_to_the_type() {
        let source = "<?php\n\nclass ArticleController extends Controller\n{\n    /**\n     * The transformer used to shape every article this controller returns.\n     */\n    protected $transformer;\n\n    public function index()\n    {\n        return $this->respond();\n    }\n\n    // Articles are listed newest first, whatever the filter says.\n    protected $order = 'desc';\n\n    public function show()\n    {\n        return $this->respond();\n    }\n}\n";
        let comments = found("ArticleController.php", source);
        assert_eq!(comments.len(), 2);
        for comment in &comments {
            assert_eq!(comment.definition.as_deref(), Some("ArticleController"));
        }
    }

    #[test]
    fn a_php_file_s_header_after_its_opening_tag_documents_the_file() {
        let source = "<?php\n\n/*\n * Custom JWT middleware: the package's token name cannot be configured.\n */\n\nnamespace App\\Http;\n\nclass Auth {}\n";
        assert_eq!(found("Auth.php", source)[0].placement, Placement::File);
    }

    #[test]
    fn framework_section_banners_are_recognized() {
        let laravel = "/*\n|--------------------------------------------------------------------------\n| Web Routes\n|--------------------------------------------------------------------------\n|\n| Here is where you can register web routes for your application.\n*/";
        assert!(banner(laravel));
        assert!(banner(&laravel.replace("\n|", "\n |")));
        assert!(!banner(
            "// Retry once: the first request is often refused.\n// | a | b |"
        ));
    }

    #[test]
    fn sphinx_version_notes_are_left_out_of_a_docstring() {
        let source = "def open_resource(name, mode=\"rb\"):\n    \"\"\"Open a resource file relative to the root path.\n\n    .. versionchanged:: 3.1\n        Added the ``encoding`` parameter.\n\n    :param name: Path to the resource.\n    \"\"\"\n    return open(name, mode)\n\n\ndef legacy():\n    \"\"\"\n    .. deprecated:: 2.3\n        Use ``open_resource`` instead.\n    \"\"\"\n    return None\n";
        let comments = found("app.py", source);
        assert_eq!(
            comments.len(),
            1,
            "a docstring of version notes only is left out"
        );
        assert!(!comments[0].text.contains("versionchanged"));
        assert!(!comments[0].text.contains("Added the"));
        assert!(comments[0].text.contains(":param name:"));
    }

    #[test]
    fn a_comment_that_closes_a_block_is_shown_with_the_lines_above() {
        let source = "function run() {\n  start();\n  stop();\n  // done\n}\n";
        let comments = found("run.js", source);
        assert_eq!(comments[0].code, "  start();\n  stop();");
    }
}
