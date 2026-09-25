//! Documentation formats besides Markdown, read as Markdown with the file's
//! own lines, so sections, outlines and findings point at the lines a person
//! edits. MDX drops its imports, exports, comments and component markup but
//! keeps the text components show. reStructuredText section titles become
//! `#` headings by the order their adornment styles appear; comments are
//! dropped, and code directives and literal blocks are fenced. AsciiDoc
//! titles become `#` headings; comments, attribute entries and block
//! attributes are dropped, and listing, literal and passthrough blocks are
//! fenced. Paths that includes and images name become code spans, so the
//! staleness candidates see them.
mod asciidoc;
mod mdx;
mod rst;

use asciidoc::asciidoc;
use mdx::mdx;
use rst::rst;
use std::{borrow::Cow, path::Path};

/// How a documentation file is written, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Mdx,
    Rst,
    AsciiDoc,
}

/// Extensions of the documentation formats read.
pub const EXTENSIONS: &[&str] = &["md", "markdown", "mdx", "rst", "adoc", "asciidoc"];

impl Format {
    pub fn of(path: &Path) -> Self {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match extension.as_str() {
            "mdx" => Self::Mdx,
            "rst" => Self::Rst,
            "adoc" | "asciidoc" => Self::AsciiDoc,
            _ => Self::Markdown,
        }
    }

    pub fn language(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::Mdx => "MDX",
            Self::Rst => "reStructuredText",
            Self::AsciiDoc => "AsciiDoc",
        }
    }
}

/// The file's text as Markdown, one line for each of the file's lines.
pub fn view<'a>(path: &Path, source: &'a str) -> Cow<'a, str> {
    let lines: Vec<&str> = source.lines().collect();
    let out = match Format::of(path) {
        Format::Markdown => return Cow::Borrowed(source),
        Format::Mdx => mdx(&lines),
        Format::Rst => rst(&lines),
        Format::AsciiDoc => asciidoc(&lines),
    };
    debug_assert_eq!(out.len(), lines.len());
    let mut text = out.join("\n");
    if source.ends_with('\n') {
        text.push('\n');
    }
    Cow::Owned(text)
}

/// `text` with each run of blank lines as one blank line, for sending: the
/// markup a view drops leaves blank lines that carry nothing.
pub fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = false;
    for line in text.lines() {
        let empty = line.trim().is_empty();
        if empty && blank {
            continue;
        }
        blank = empty;
        out.push_str(if empty { "" } else { line });
        out.push('\n');
    }
    if !text.ends_with('\n') {
        out.pop();
    }
    out
}

/// The zero-based line after a leading `---` frontmatter block, or 0.
fn frontmatter_end(lines: &[&str]) -> usize {
    if lines.first().map(|l| l.trim()) != Some("---") {
        return 0;
    }
    lines
        .iter()
        .skip(1)
        .position(|l| l.trim() == "---")
        .map_or(0, |close| close + 2)
}

fn fence_marker(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_start();
    ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::docs::markdown;

    /// The view of `source`, checked to keep the file's lines.
    pub(in crate::docs::format) fn view_of(path: &str, source: &str) -> String {
        let text = view(Path::new(path), source).into_owned();
        assert_eq!(
            text.lines().count(),
            source.lines().count(),
            "lines stay the file's"
        );
        text
    }

    /// The lines of the view of `source`, with those at `blank` checked to
    /// read as blank.
    pub(in crate::docs::format) fn view_lines(
        path: &str,
        source: &str,
        blank: &[usize],
    ) -> Vec<String> {
        let lines: Vec<String> = view_of(path, source).lines().map(str::to_string).collect();
        for &index in blank {
            assert_eq!(lines[index], "", "line {index} of the view");
        }
        lines
    }

    pub(in crate::docs::format) fn headings(
        path: &str,
        source: &str,
    ) -> Vec<(usize, usize, String)> {
        markdown::headings(&view_of(path, source))
            .into_iter()
            .map(|h| (h.line, h.level, h.text))
            .collect()
    }

    #[test]
    fn formats_follow_the_extension() {
        for (path, format) in [
            ("README.md", Format::Markdown),
            ("docs/a.MDX", Format::Mdx),
            ("docs/index.rst", Format::Rst),
            ("docs/modules/ROOT/pages/a.adoc", Format::AsciiDoc),
            ("guide.asciidoc", Format::AsciiDoc),
        ] {
            assert_eq!(Format::of(Path::new(path)), format, "{path}");
        }
        let source = "# Title\n\nText\n";
        assert!(matches!(view(Path::new("a.md"), source), Cow::Borrowed(_)));
    }
}
