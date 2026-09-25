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

// MDX

fn mdx(lines: &[&str]) -> Vec<String> {
    let body = frontmatter_end(lines);
    let mut out: Vec<String> = lines[..body].iter().map(|l| (*l).to_string()).collect();
    let mut fence: Option<&str> = None;
    let mut esm_depth: Option<i32> = None;
    let mut jsx = Jsx::default();
    for line in &lines[body..] {
        if let Some(open) = fence {
            if line.trim_start().starts_with(open) {
                fence = None;
            }
            out.push((*line).to_string());
            continue;
        }
        if let Some(depth) = esm_depth.as_mut() {
            *depth += brackets(line);
            if *depth <= 0 && !continues(line) {
                esm_depth = None;
            }
            out.push(String::new());
            continue;
        }
        if !jsx.open() {
            if let Some(open) = fence_marker(line) {
                fence = Some(open);
                out.push((*line).to_string());
                continue;
            }
            if ["import ", "export "].iter().any(|k| line.starts_with(k)) {
                let depth = brackets(line);
                if depth > 0 || continues(line) {
                    esm_depth = Some(depth);
                }
                out.push(String::new());
                continue;
            }
        }
        out.push(jsx.strip(line));
    }
    out
}

/// Opening minus closing brackets outside string literals.
fn brackets(line: &str) -> i32 {
    let mut depth = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in line.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Whether a statement goes on after this line.
fn continues(line: &str) -> bool {
    let trimmed = line.trim_end();
    [",", "=", "=>", "(", "{", "[", "+", "&&", "||", "?", ":"]
        .iter()
        .any(|end| trimmed.ends_with(end))
}

/// Attributes that hold links, styles or identifiers rather than text a
/// reader sees.
const MARKUP_ATTRIBUTES: &[&str] = &[
    "className",
    "class",
    "style",
    "href",
    "src",
    "id",
    "key",
    "type",
    "variant",
    "size",
    "icon",
    "color",
    "language",
    "lang",
    "filename",
    "highlight",
    "target",
    "rel",
    "width",
    "height",
    "layout",
];

/// Keys of the objects a component documents, such as the rows of a
/// properties table, whose values name code: kept as code spans, so two
/// tables of different properties do not read as the same descriptions.
const NAMING_KEYS: &[&str] = &["name", "type"];

/// Component markup being read across lines: `{/* */}` comments and tags
/// with their attributes, whose prose string values are kept as text.
#[derive(Default)]
struct Jsx {
    comment: bool,
    tag: Option<Tag>,
}

#[derive(Default)]
struct Tag {
    /// Depth of `{}` expressions inside the tag.
    depth: usize,
    quote: Option<char>,
    escaped: bool,
    literal: String,
    /// The attribute or key the current literal belongs to.
    name: String,
    word: String,
    /// Whitespace since the last word character, so the next one starts a word.
    gap: bool,
}

impl Jsx {
    fn open(&self) -> bool {
        self.comment || self.tag.is_some()
    }

    /// The line's text outside component markup, with the prose that
    /// attributes carry.
    fn strip(&mut self, line: &str) -> String {
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::new();
        let mut markup = self.open();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if self.comment {
                if chars[i..].starts_with(&['*', '/', '}']) {
                    self.comment = false;
                    i += 3;
                } else {
                    i += 1;
                }
                continue;
            }
            if let Some(tag) = self.tag.as_mut() {
                if tag.read(c, &mut out) {
                    self.tag = None;
                }
                i += 1;
                continue;
            }
            if chars[i..].starts_with(&['{', '/', '*']) {
                self.comment = true;
                markup = true;
                i += 3;
            } else if c == '`' {
                // A code span is copied as written, markup and all.
                let run = chars[i..].iter().take_while(|&&c| c == '`').count();
                let close = (i + run..chars.len())
                    .find(|&j| chars[j..].iter().take_while(|&&c| c == '`').count() == run);
                let end = close.map_or(chars.len(), |j| j + run);
                out.extend(&chars[i..end]);
                i = end;
            } else if c == '<' && tag_start(&chars[i + 1..]) {
                self.tag = Some(Tag::default());
                markup = true;
                i += 1;
            } else {
                out.push(c);
                i += 1;
            }
        }
        if !markup {
            return out;
        }
        let kept = out.trim_end();
        if kept.trim().is_empty() {
            String::new()
        } else if kept.trim_start().starts_with('#') && !line.trim_start().starts_with('#') {
            // Text a component carried never becomes a heading.
            format!("    {}", kept.trim_start())
        } else {
            kept.to_string()
        }
    }
}

/// Whether `<` followed by `rest` opens or closes a component or element,
/// rather than comparing values or wrapping a link.
fn tag_start(rest: &[char]) -> bool {
    match rest.first() {
        Some('/' | '>') => true,
        Some(c) if c.is_ascii_alphabetic() => {
            let scheme: String = rest
                .iter()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            !rest[scheme.len()..].starts_with(&[':', '/', '/'])
        }
        _ => false,
    }
}

impl Tag {
    /// Read one character of a tag; true when the tag closes.
    fn read(&mut self, c: char, out: &mut String) -> bool {
        if let Some(q) = self.quote {
            if self.escaped {
                self.escaped = false;
                self.literal.push(c);
            } else if c == '\\' {
                self.escaped = true;
            } else if c == q {
                self.quote = None;
                let literal = std::mem::take(&mut self.literal);
                let kept = if prose(&literal) && !MARKUP_ATTRIBUTES.contains(&self.name.as_str()) {
                    Some(literal.trim().to_string())
                } else if self.depth > 0
                    && NAMING_KEYS.contains(&self.name.as_str())
                    && !literal.trim().is_empty()
                    && !literal.contains('`')
                {
                    // The name and type of a documented property, as code.
                    Some(format!("`{}`", literal.trim()))
                } else {
                    None
                };
                if let Some(kept) = kept {
                    if !out.is_empty() && !out.ends_with(' ') {
                        out.push(' ');
                    }
                    out.push_str(&kept);
                }
            } else {
                self.literal.push(c);
            }
            return false;
        }
        match c {
            '"' | '\'' | '`' => {
                self.quote = Some(c);
                self.name = std::mem::take(&mut self.word);
            }
            '{' => self.depth += 1,
            '}' => self.depth = self.depth.saturating_sub(1),
            '>' if self.depth == 0 => return true,
            c if c.is_alphanumeric() || c == '_' || c == '-' => {
                if std::mem::take(&mut self.gap) {
                    self.word.clear();
                }
                self.word.push(c);
            }
            '=' | ':' => {}
            c if c.is_whitespace() => self.gap = true,
            _ => self.word.clear(),
        }
        false
    }
}

/// A string a reader sees: words, not a single token or a value.
fn prose(literal: &str) -> bool {
    literal.trim().contains(' ') && literal.chars().any(char::is_alphabetic)
}

// reStructuredText

/// Directives whose content is code or data rather than prose.
const CODE_DIRECTIVES: &[&str] = &[
    "code-block",
    "code",
    "sourcecode",
    "parsed-literal",
    "math",
    "raw",
    "doctest",
    "testcode",
    "testoutput",
    "testsetup",
    "testcleanup",
    "ipython",
    "jupyter-execute",
    "csv-table",
    "graphviz",
    "digraph",
    "graph",
    "mermaid",
    "tabs-code",
];

/// Directives whose argument names a file of the repository.
const PATH_DIRECTIVES: &[&str] = &["include", "literalinclude", "image", "figure"];

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// The character of a line that is one punctuation character repeated.
fn adornment(line: &str) -> Option<char> {
    let line = line.trim_end();
    let first = line.chars().next()?;
    (line.len() >= 2 && first.is_ascii_punctuation() && line.chars().all(|c| c == first))
        .then_some(first)
}

/// The end (exclusive) of the lines indented past `column` from `start`,
/// without trailing blank lines.
fn indented_end(lines: &[&str], start: usize, column: usize) -> usize {
    let mut end = start;
    for (j, line) in lines.iter().enumerate().skip(start) {
        if blank(line) {
            continue;
        }
        if indent(line) <= column {
            break;
        }
        end = j + 1;
    }
    end
}

struct Rst<'a> {
    lines: &'a [&'a str],
    out: Vec<String>,
    code: Vec<bool>,
    /// Adornment styles in the order they first appear: character and
    /// whether the title is overlined.
    styles: Vec<(char, bool)>,
}

fn rst(lines: &[&str]) -> Vec<String> {
    let mut doc = Rst {
        lines,
        out: lines.iter().map(|l| (*l).to_string()).collect(),
        code: vec![false; lines.len()],
        styles: Vec::new(),
    };
    let mut i = 0;
    while i < lines.len() {
        i = doc.line(i);
    }
    for (line, code) in doc.out.iter_mut().zip(&doc.code) {
        if !code {
            *line = line.replace("``", "`");
        }
    }
    doc.out
}

impl Rst<'_> {
    /// Read the construct starting at line `i`; returns the next line to read.
    fn line(&mut self, i: usize) -> usize {
        let lines = self.lines;
        let line = lines[i];
        if blank(line) {
            return i + 1;
        }
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("..")
            && (rest.is_empty() || rest.starts_with(' '))
        {
            return self.explicit(i, rest.trim());
        }
        if indent(line) == 0
            && let Some(next) = self.title(i)
        {
            return next;
        }
        if line.trim_end().ends_with("::") {
            let end = self.literal(i, indent(line));
            if end > i + 1 {
                return end;
            }
        }
        i + 1
    }

    /// A section title, overlined or not, or a transition.
    fn title(&mut self, i: usize) -> Option<usize> {
        let lines = self.lines;
        let line = lines[i];
        let after_blank = i == 0 || blank(lines[i - 1]);
        match adornment(line) {
            Some(c) => {
                let (Some(text), Some(under)) = (lines.get(i + 1), lines.get(i + 2)) else {
                    return (after_blank && lines.get(i + 1).is_none_or(|l| blank(l)))
                        .then(|| self.blank_line(i, i + 1));
                };
                if after_blank
                    && !blank(text)
                    && adornment(text).is_none()
                    && adornment(under) == Some(c)
                {
                    self.heading(i + 1, c, true);
                    self.out[i].clear();
                    self.out[i + 2].clear();
                    return Some(i + 3);
                }
                (after_blank && blank(text)).then(|| self.blank_line(i, i + 1))
            }
            None => {
                let under = lines.get(i + 1)?;
                let c = adornment(under)?;
                let width = line.trim().chars().count();
                let long_enough = under.trim_end().len() >= width.min(4);
                (after_blank && long_enough).then(|| {
                    self.heading(i, c, false);
                    self.out[i + 1].clear();
                    i + 2
                })
            }
        }
    }

    fn blank_line(&mut self, i: usize, next: usize) -> usize {
        self.out[i].clear();
        next
    }

    fn heading(&mut self, i: usize, c: char, overlined: bool) {
        let style = (c, overlined);
        let level = match self.styles.iter().position(|s| *s == style) {
            Some(p) => p + 1,
            None => {
                self.styles.push(style);
                self.styles.len()
            }
        }
        .min(crate::docs::markdown::DEEPEST_HEADING);
        self.out[i] = format!("{} {}", "#".repeat(level), self.lines[i].trim());
    }

    /// A comment, directive, target or substitution starting with `..`.
    fn explicit(&mut self, i: usize, rest: &str) -> usize {
        let lines = self.lines;
        let column = indent(lines[i]);
        let end = indented_end(lines, i + 1, column);
        // Targets, footnotes, citations and substitutions stay as written.
        if rest.starts_with(['_', '[', '|']) {
            return i + 1;
        }
        let directive = rest
            .split_once("::")
            .map(|(name, argument)| (name.trim(), argument.trim()))
            .filter(|(name, _)| !name.is_empty() && !name.contains(char::is_whitespace));
        let Some((name, argument)) = directive else {
            // A comment: every line of it is dropped.
            for line in &mut self.out[i..end.max(i + 1)] {
                line.clear();
            }
            return end.max(i + 1);
        };
        // Options follow the directive line up to the first blank line.
        let options = (i + 1..end)
            .take_while(|&j| !blank(lines[j]) && lines[j].trim_start().starts_with(':'))
            .count();
        if PATH_DIRECTIVES.contains(&name) && !argument.is_empty() && !argument.contains(' ') {
            let marker = &lines[i][..lines[i].find("::").unwrap_or(0) + 2];
            self.out[i] = format!("{marker} `{argument}`");
        }
        if CODE_DIRECTIVES.contains(&name) {
            for line in &mut self.out[i + 1..i + 1 + options] {
                line.clear();
            }
            // The fence names its language, as Markdown's would.
            let language = match name {
                "code-block" | "code" | "sourcecode" => argument.split_whitespace().next(),
                _ if name.starts_with("test")
                    || ["doctest", "ipython", "jupyter-execute"].contains(&name) =>
                {
                    Some("python")
                }
                _ => Some(name),
            };
            self.fence(i + 1 + options, end, column, language.unwrap_or(""));
            return end.max(i + 1);
        }
        i + 1
    }

    /// An indented literal block after a paragraph ending in `::`; returns
    /// the line after it, or `i + 1` when none follows.
    fn literal(&mut self, i: usize, column: usize) -> usize {
        let lines = self.lines;
        if lines.get(i + 1).is_none_or(|l| !blank(l)) {
            return i + 1;
        }
        let end = indented_end(lines, i + 1, column);
        if end <= i + 1 {
            return i + 1;
        }
        self.fence(i + 1, end, column, "");
        end
    }

    /// Fence the code in `start..end`: its first blank line opens the fence
    /// and the blank line after it closes it; without those lines, the code
    /// is indented past where a heading can start.
    fn fence(&mut self, start: usize, end: usize, column: usize, language: &str) {
        let lines = self.lines;
        let Some(open) = (start..end).find(|&j| blank(lines[j])) else {
            return;
        };
        for j in open..end {
            self.code[j] = true;
        }
        let closable = end == lines.len() || blank(lines[end]);
        if closable {
            let pad = " ".repeat(column);
            self.out[open] = format!("{pad}```{language}");
            if end < lines.len() {
                self.out[end] = format!("{pad}```");
                self.code[end] = true;
            }
        } else {
            for (out, line) in self.out[open..end].iter_mut().zip(&lines[open..end]) {
                if !blank(line) {
                    *out = format!("    {line}");
                }
            }
        }
    }
}

// AsciiDoc

#[derive(Clone, Copy, PartialEq)]
enum Block {
    Code,
    Comment,
    Open,
}

/// A delimited block's kind from its delimiter line.
fn delimiter(line: &str) -> Option<Block> {
    if line == "--" || ["|===", ",===", ":===", "!==="].contains(&line) {
        return Some(Block::Open);
    }
    let first = line.chars().next()?;
    if line.len() < 4 || !line.chars().all(|c| c == first) {
        return None;
    }
    match first {
        '-' | '.' | '+' => Some(Block::Code),
        '/' => Some(Block::Comment),
        '=' | '*' | '_' => Some(Block::Open),
        _ => None,
    }
}

/// Whether the delimited block opening at `i` holds only include
/// directives: a listing that pulls in a file names it rather than showing
/// code, so its includes are read like prose.
fn only_includes(lines: &[&str], i: usize) -> bool {
    let open = lines[i].trim_end();
    let Some(close) = (i + 1..lines.len()).find(|&j| lines[j].trim_end() == open) else {
        return false;
    };
    let inner = &lines[i + 1..close];
    inner.iter().any(|l| l.starts_with("include::"))
        && inner
            .iter()
            .all(|l| l.trim().is_empty() || l.starts_with("include::"))
}

fn asciidoc(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut block: Option<(&str, Block)> = None;
    let mut fence: Option<&str> = None;
    let mut includes: Option<&str> = None;
    // The language a `[source,lang]` attribute line gives the next block.
    let mut language = "";
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_end();
        if let Some(open) = includes {
            if trimmed == open {
                includes = None;
                out.push(String::new());
            } else {
                out.push(asciidoc_line(trimmed));
            }
            continue;
        }
        if let Some(open) = fence {
            if line.trim_start().starts_with(open) {
                fence = None;
            }
            out.push((*line).to_string());
            continue;
        }
        if let Some((open, kind)) = block {
            let closes = trimmed == open;
            if closes {
                block = None;
            }
            out.push(match kind {
                Block::Code if closes => "```".into(),
                Block::Code => (*line).to_string(),
                _ => String::new(),
            });
            continue;
        }
        if let Some(kind) = delimiter(trimmed) {
            if kind == Block::Code && only_includes(lines, i) {
                includes = Some(trimmed);
                out.push(String::new());
                continue;
            }
            if kind != Block::Open {
                block = Some((trimmed, kind));
            }
            out.push(if kind == Block::Code {
                format!("```{}", std::mem::take(&mut language))
            } else {
                String::new()
            });
            continue;
        }
        if let Some(attributes) = trimmed.strip_prefix("[source,") {
            language = attributes.split([',', ']']).next().unwrap_or("").trim();
        } else if !trimmed.is_empty() && !trimmed.starts_with('.') {
            language = "";
        }
        if let Some(open) = fence_marker(line) {
            fence = Some(open);
            out.push((*line).to_string());
            continue;
        }
        out.push(asciidoc_line(trimmed));
    }
    out
}

/// One AsciiDoc line outside delimited blocks.
fn asciidoc_line(line: &str) -> String {
    let equals = line.chars().take_while(|c| *c == '=').count();
    if (1..=crate::docs::markdown::DEEPEST_HEADING).contains(&equals)
        && line[equals..].starts_with(' ')
    {
        return format!("{}{}", "#".repeat(equals), &line[equals..]);
    }
    let attribute_entry = line
        .strip_prefix(':')
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(name, value)| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '!'))
                && (value.is_empty() || value.starts_with(' '))
        });
    let block_attributes = line.starts_with('[') && line.ends_with(']');
    if attribute_entry || block_attributes || line.starts_with("//") {
        return String::new();
    }
    if let Some(title) = line.strip_prefix('.')
        && title.starts_with(|c: char| c.is_alphanumeric())
    {
        return title.to_string();
    }
    for macro_name in ["include::", "image::"] {
        if let Some(rest) = line.strip_prefix(macro_name)
            && let Some((target, attributes)) = rest.split_once('[')
            && !target.is_empty()
            && !target.contains([' ', '{'])
        {
            return format!("{macro_name}`{target}`[{attributes}");
        }
    }
    line.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::markdown;

    fn view_of(path: &str, source: &str) -> String {
        let text = view(Path::new(path), source).into_owned();
        assert_eq!(
            text.lines().count(),
            source.lines().count(),
            "lines stay the file's"
        );
        text
    }

    fn headings(path: &str, source: &str) -> Vec<(usize, usize, String)> {
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

    #[test]
    fn mdx_drops_imports_exports_comments_and_component_markup() {
        let source = "---\ntitle: Tools\n---\nimport { Note } from '@/components';\nimport {\n  Tabs,\n  Tab,\n} from 'nextra';\nexport const meta = {\n  a: 1,\n};\n\n# Tools\n\n{/* hidden\n   note */}\n<Note type=\"warning\">\n  Tools run on the server.\n</Note>\n<Snippet text=\"pnpm add ai\" />\nUse `Array<string>` here; x < y.\n";
        let text = view_of("docs/tools.mdx", source);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(&lines[..3], ["---", "title: Tools", "---"]);
        assert!(lines[3..12].iter().all(|l| l.is_empty()), "{lines:?}");
        assert_eq!(lines[12], "# Tools");
        assert_eq!((lines[14], lines[15]), ("", ""));
        assert_eq!(lines[16], "");
        assert_eq!(lines[17], "  Tools run on the server.");
        assert_eq!(lines[18], "");
        assert_eq!(lines[19], "pnpm add ai");
        assert_eq!(lines[20], "Use `Array<string>` here; x < y.");
    }

    #[test]
    fn mdx_keeps_prose_that_multiline_components_carry() {
        let source = "## Parameters\n\n<PropertiesTable\n  content={[\n    {\n      name: 'model',\n      type: 'EmbeddingModel',\n      description:\n        'The embedding model to use.',\n    },\n  ]}\n/>\n\n```tsx\n<Chat />\n# not a heading\n```\n<Card className=\"flex items-center\" title=\"Get started now\" href=\"/docs\" />\n";
        let text = view_of("docs/embed.mdx", source);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            &lines[5..9],
            [
                "`model`",
                "`EmbeddingModel`",
                "",
                "The embedding model to use."
            ],
            "a property keeps its name and type as code"
        );
        for (n, line) in lines.iter().enumerate().take(12).skip(2) {
            if !(5..=8).contains(&n) {
                assert!(line.is_empty(), "line {}: {line:?}", n + 1);
            }
        }
        assert_eq!(
            &lines[13..17],
            ["```tsx", "<Chat />", "# not a heading", "```"]
        );
        assert_eq!(lines[17], "Get started now");
        assert_eq!(
            headings("docs/embed.mdx", source),
            [(1, 2, "Parameters".to_string())]
        );
    }

    #[test]
    fn rst_titles_become_headings_by_the_order_of_their_styles() {
        let source = "=====\nFlask\n=====\n\nIntro text.\n\nInstall\n-------\n\nRun it.\n\nPython Version\n~~~~~~~~~~~~~~\n\nText.\n\n----\n\nUsage\n-----\n\nShort\n--\n";
        assert_eq!(
            headings("docs/index.rst", source),
            [
                (2, 1, "Flask".to_string()),
                (7, 2, "Install".to_string()),
                (12, 3, "Python Version".to_string()),
                (19, 2, "Usage".to_string()),
            ]
        );
        let text = view_of("docs/index.rst", source);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!((lines[0], lines[2], lines[7], lines[16]), ("", "", "", ""));
        assert_eq!(
            (lines[21], lines[22]),
            ("Short", "--"),
            "an underline shorter than 4 and its title is text"
        );
    }

    #[test]
    fn rst_comments_drop_and_code_is_fenced() {
        let source = "Setup\n=====\n\n.. This comment\n   spans lines.\n\nRun this::\n\n   # install\n   pip install flask\n\nThen:\n\n.. code-block:: python\n   :caption: app.py\n\n   # create the app\n   app = Flask(__name__)\n\n.. note::\n\n   Use ``flask run``, see :file:`src/app.py`.\n\n.. literalinclude:: ../examples/app.py\n   :lines: 1-4\n";
        let text = view_of("docs/setup.rst", source);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "# Setup");
        assert_eq!((lines[3], lines[4]), ("", ""));
        assert_eq!(
            &lines[6..11],
            [
                "Run this::",
                "```",
                "   # install",
                "   pip install flask",
                "```"
            ]
        );
        assert_eq!(lines[13], ".. code-block:: python");
        assert_eq!(lines[14], "");
        assert_eq!(
            &lines[15..19],
            [
                "```python",
                "   # create the app",
                "   app = Flask(__name__)",
                "```"
            ]
        );
        assert_eq!(lines[21], "   Use `flask run`, see :file:`src/app.py`.");
        assert_eq!(lines[23], ".. literalinclude:: `../examples/app.py`");
        assert_eq!(
            headings("docs/setup.rst", source),
            [(1, 1, "Setup".to_string())]
        );
        let sections = markdown::parse(&text).sections;
        assert_eq!(sections.len(), 1);
        assert!(!sections[0].text.contains("This comment"));
    }

    #[test]
    fn asciidoc_titles_blocks_and_comments() {
        let source = "= Guide\n:toc: left\n:source-highlighter: rouge\n\n== Build\n\n// a comment\n[source,sh]\n----\n# not a heading\n./gradlew build\n----\n\n////\nhidden\n////\n\n.Example title\n====\nExample text.\n====\n\ninclude::partials/setup.adoc[]\n\n=== Deeper\n";
        assert_eq!(
            headings("docs/guide.adoc", source),
            [
                (1, 1, "Guide".to_string()),
                (5, 2, "Build".to_string()),
                (25, 3, "Deeper".to_string()),
            ]
        );
        let text = view_of("docs/guide.adoc", source);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!((lines[1], lines[2], lines[6], lines[7]), ("", "", "", ""));
        assert_eq!(
            &lines[8..12],
            ["```sh", "# not a heading", "./gradlew build", "```"]
        );
        assert_eq!((lines[13], lines[14], lines[15]), ("", "", ""));
        assert_eq!(lines[17], "Example title");
        assert_eq!((lines[18], lines[19], lines[20]), ("", "Example text.", ""));
        assert_eq!(lines[22], "include::`partials/setup.adoc`[]");
    }

    #[test]
    fn asciidoc_listings_of_includes_name_their_files() {
        let source = "[source,java]\n----\ninclude::complete/src/Greeting.java[]\n----\n\n[source,sh]\n----\ninclude::run.sh[]\n./mvnw spring-boot:run\n----\n";
        let text = view_of("README.adoc", source);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            &lines[..4],
            ["", "", "include::`complete/src/Greeting.java`[]", ""]
        );
        assert_eq!(
            &lines[6..10],
            [
                "```sh",
                "include::run.sh[]",
                "./mvnw spring-boot:run",
                "```"
            ],
            "a listing with code stays code"
        );
    }
}
