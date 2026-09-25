//! MDX read as Markdown: imports, exports, comments and component markup
//! are dropped, but the prose components carry is kept.
use super::{fence_marker, frontmatter_end};

pub(super) fn mdx(lines: &[&str]) -> Vec<String> {
    let body = frontmatter_end(lines);
    let mut out: Vec<String> = lines[..body].iter().map(|l| (*l).to_string()).collect();
    let mut reader = Mdx::default();
    out.extend(lines[body..].iter().map(|line| reader.line(line)));
    out
}

/// What the lines read so far leave open.
#[derive(Default)]
struct Mdx {
    /// The marker of a code fence being read.
    fence: Option<&'static str>,
    /// The bracket depth of an import or export statement being read.
    statement: Option<i32>,
    jsx: Jsx,
}

impl Mdx {
    /// One line of the body as Markdown.
    fn line(&mut self, line: &str) -> String {
        if let Some(open) = self.fence {
            if line.trim_start().starts_with(open) {
                self.fence = None;
            }
            return line.to_string();
        }
        if let Some(depth) = self.statement.as_mut() {
            *depth += brackets(line);
            if *depth <= 0 && !continues(line) {
                self.statement = None;
            }
            return String::new();
        }
        if !self.jsx.open() {
            if let Some(open) = fence_marker(line) {
                self.fence = Some(open);
                return line.to_string();
            }
            if ["import ", "export "].iter().any(|k| line.starts_with(k)) {
                let depth = brackets(line);
                if depth > 0 || continues(line) {
                    self.statement = Some(depth);
                }
                return String::new();
            }
        }
        self.jsx.strip(line)
    }
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
            i = self.read(&chars, i, &mut out, &mut markup);
        }
        if markup { kept_text(&out, line) } else { out }
    }

    /// Read the character at `i`, copying what a reader sees to `out` and
    /// setting `markup` when a comment or tag opens; returns the next index.
    fn read(&mut self, chars: &[char], i: usize, out: &mut String, markup: &mut bool) -> usize {
        if self.comment {
            if chars[i..].starts_with(&['*', '/', '}']) {
                self.comment = false;
                return i + 3;
            }
            return i + 1;
        }
        if let Some(tag) = self.tag.as_mut() {
            if tag.read(chars[i], out) {
                self.tag = None;
            }
            return i + 1;
        }
        if chars[i..].starts_with(&['{', '/', '*']) {
            self.comment = true;
            *markup = true;
            return i + 3;
        }
        if chars[i] == '`' {
            return code_span(chars, i, out);
        }
        if chars[i] == '<' && tag_start(&chars[i + 1..]) {
            self.tag = Some(Tag::default());
            *markup = true;
            return i + 1;
        }
        out.push(chars[i]);
        i + 1
    }
}

/// A code span starting at `i`, copied as written, markup and all; returns
/// the index after it.
fn code_span(chars: &[char], i: usize, out: &mut String) -> usize {
    let run = chars[i..].iter().take_while(|&&c| c == '`').count();
    let close = (i + run..chars.len())
        .find(|&j| chars[j..].iter().take_while(|&&c| c == '`').count() == run);
    let end = close.map_or(chars.len(), |j| j + run);
    out.extend(&chars[i..end]);
    end
}

/// The text of a line that held component markup: nothing when only markup
/// was there, and never a heading made of text a component carried.
fn kept_text(out: &str, line: &str) -> String {
    let kept = out.trim_end();
    if kept.trim().is_empty() {
        String::new()
    } else if kept.trim_start().starts_with('#') && !line.trim_start().starts_with('#') {
        format!("    {}", kept.trim_start())
    } else {
        kept.to_string()
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
            self.quoted(q, c, out);
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

    /// Read one character of a string literal quoted with `q`; when it
    /// closes, add what it carries to `out`.
    fn quoted(&mut self, q: char, c: char, out: &mut String) {
        if self.escaped {
            self.escaped = false;
            self.literal.push(c);
        } else if c == '\\' {
            self.escaped = true;
        } else if c == q {
            self.quote = None;
            let literal = std::mem::take(&mut self.literal);
            if let Some(kept) = self.kept(&literal) {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(&kept);
            }
        } else {
            self.literal.push(c);
        }
    }

    /// What a closed literal adds to the text: prose a reader sees, or the
    /// name and type of a documented property, as code.
    fn kept(&self, literal: &str) -> Option<String> {
        let name = self.name.as_str();
        if prose(literal) && !MARKUP_ATTRIBUTES.contains(&name) {
            Some(literal.trim().to_string())
        } else if self.depth > 0
            && NAMING_KEYS.contains(&name)
            && !literal.trim().is_empty()
            && !literal.contains('`')
        {
            Some(format!("`{}`", literal.trim()))
        } else {
            None
        }
    }
}

/// A string a reader sees: words, not a single token or a value.
fn prose(literal: &str) -> bool {
    literal.trim().contains(' ') && literal.chars().any(char::is_alphabetic)
}

#[cfg(test)]
mod tests {
    use crate::docs::format::tests::{headings, view_of};

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
}
