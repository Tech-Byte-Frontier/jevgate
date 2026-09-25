//! AsciiDoc read as Markdown: titles become `#` headings; comments,
//! attribute entries and block attributes are dropped, and listing, literal
//! and passthrough blocks are fenced. Paths that includes and images name
//! become code spans.
use super::fence_marker;

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

pub(super) fn asciidoc(lines: &[&str]) -> Vec<String> {
    let mut reader = AsciiDoc::default();
    (0..lines.len()).map(|i| reader.line(lines, i)).collect()
}

/// What the lines read so far leave open.
#[derive(Default)]
struct AsciiDoc<'a> {
    /// A delimited block being read, by its delimiter line and kind.
    block: Option<(&'a str, Block)>,
    /// The marker of a Markdown-style code fence being read.
    fence: Option<&'static str>,
    /// A listing of include directives being read, by its delimiter line.
    includes: Option<&'a str>,
    /// The language a `[source,lang]` attribute line gives the next block.
    language: &'a str,
}

impl<'a> AsciiDoc<'a> {
    /// Line `i` as Markdown.
    fn line(&mut self, lines: &[&'a str], i: usize) -> String {
        let line = lines[i];
        let trimmed = line.trim_end();
        if let Some(open) = self.includes {
            if trimmed == open {
                self.includes = None;
                return String::new();
            }
            return asciidoc_line(trimmed);
        }
        if let Some(open) = self.fence {
            if line.trim_start().starts_with(open) {
                self.fence = None;
            }
            return line.to_string();
        }
        if let Some((open, kind)) = self.block {
            return self.inside(open, kind, line);
        }
        if let Some(kind) = delimiter(trimmed) {
            return self.open(lines, i, kind);
        }
        if let Some(attributes) = trimmed.strip_prefix("[source,") {
            self.language = attributes.split([',', ']']).next().unwrap_or("").trim();
        } else if !trimmed.is_empty() && !trimmed.starts_with('.') {
            self.language = "";
        }
        if let Some(open) = fence_marker(line) {
            self.fence = Some(open);
            return line.to_string();
        }
        asciidoc_line(trimmed)
    }

    /// A line inside the delimited block `open`: code is kept and fenced,
    /// other blocks' lines are dropped.
    fn inside(&mut self, open: &str, kind: Block, line: &str) -> String {
        let closes = line.trim_end() == open;
        if closes {
            self.block = None;
        }
        match kind {
            Block::Code if closes => "```".into(),
            Block::Code => line.to_string(),
            _ => String::new(),
        }
    }

    /// The delimiter line at `i`, opening a block of `kind`.
    fn open(&mut self, lines: &[&'a str], i: usize, kind: Block) -> String {
        let delimiter = lines[i].trim_end();
        if kind == Block::Code && only_includes(lines, i) {
            self.includes = Some(delimiter);
            return String::new();
        }
        if kind != Block::Open {
            self.block = Some((delimiter, kind));
        }
        if kind == Block::Code {
            format!("```{}", std::mem::take(&mut self.language))
        } else {
            String::new()
        }
    }
}

/// One AsciiDoc line outside delimited blocks.
fn asciidoc_line(line: &str) -> String {
    let equals = line.chars().take_while(|c| *c == '=').count();
    if (1..=crate::docs::markdown::DEEPEST_HEADING).contains(&equals)
        && line[equals..].starts_with(' ')
    {
        return format!("{}{}", "#".repeat(equals), &line[equals..]);
    }
    let block_attributes = line.starts_with('[') && line.ends_with(']');
    if attribute_entry(line) || block_attributes || line.starts_with("//") {
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

/// An attribute entry, `:name: value` or `:name!:`.
fn attribute_entry(line: &str) -> bool {
    line.strip_prefix(':')
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(name, value)| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '!'))
                && (value.is_empty() || value.starts_with(' '))
        })
}

#[cfg(test)]
mod tests {
    use crate::docs::format::tests::{headings, view_of};

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
