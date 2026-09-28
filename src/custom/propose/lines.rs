//! The candidate lines of an instruction file: each list item, at any
//! depth, and each paragraph, with the heading above it and the text that
//! introduces it. Code, tables, headings, HTML comments and `@path` imports
//! hold no rule, and a line ending in `:` that opens a list introduces its
//! items rather than stating one.
use crate::{
    custom::characters::{BYTE_ORDER_MARK, printable},
    docs::markdown,
};

/// Characters of a candidate at most. A rule is a line or a list item: a
/// longer paragraph is a page of prose, which its proposal could not quote
/// whole in its guidance (at most 2,000 characters).
const MAX_CHARS: usize = 1_600;

/// Spaces a tab counts for in a list item's indentation.
const TAB_WIDTH: usize = 4;

/// One candidate: a list item or a paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Its first and last lines, one-based.
    pub start_line: usize,
    pub end_line: usize,
    /// The heading above it; empty before the first.
    pub heading: String,
    /// The text that introduces it: its parent item, or the paragraph
    /// ending in `:` right before its list.
    pub lead_in: Option<String>,
    /// Its lines joined by single spaces, without the list marker and
    /// control characters, otherwise as written.
    pub text: String,
}

/// The candidates of `source`, in order. A byte-order mark would hide the
/// first line's heading or frontmatter.
pub fn lines(source: &str) -> Vec<Line> {
    let source = source.strip_prefix(BYTE_ORDER_MARK).unwrap_or(source);
    let blanked = markdown::blank_comments(source);
    let all: Vec<&str> = blanked.lines().collect();
    let mut found = Vec::new();
    for section in markdown::parse(&blanked).sections {
        let first = section.start_line - 1 + usize::from(!section.heading.is_empty());
        let mut walk = Walk::new(&section.heading);
        for (index, line) in all.iter().enumerate().take(section.end_line).skip(first) {
            walk.read(index + 1, line);
        }
        found.extend(walk.finish());
    }
    found
}

/// What a candidate is: a list item, with the indentation of its marker, a
/// paragraph, or a quote.
#[derive(Clone, Copy, PartialEq)]
enum Block {
    Item(usize),
    Paragraph,
    Quote,
}

/// A candidate being read.
struct Open {
    line: Line,
    block: Block,
}

/// One section's lines, read in order.
struct Walk<'a> {
    heading: &'a str,
    /// The fence of the code block the walk is in.
    fence: Option<&'static str>,
    /// The list items that enclose the next line, outermost first, with the
    /// indentation of their markers.
    items: Vec<(usize, String)>,
    /// The paragraph that introduces the current list.
    introduction: Option<String>,
    open: Option<Open>,
    /// A candidate ending in `:`, kept until what follows shows whether it
    /// introduces a list.
    held: Option<Open>,
    found: Vec<Line>,
}

impl<'a> Walk<'a> {
    fn new(heading: &'a str) -> Self {
        Self {
            heading,
            fence: None,
            items: Vec::new(),
            introduction: None,
            open: None,
            held: None,
            found: Vec::new(),
        }
    }

    /// Read line `number` (one-based) of the file.
    fn read(&mut self, number: usize, raw: &str) {
        let trimmed = raw.trim_start();
        if let Some(fence) = self.fence {
            if trimmed.starts_with(fence) {
                self.fence = None;
            }
            return;
        }
        let indent = indentation(raw);
        if let Some(fence) = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f)) {
            self.interrupt(indent);
            self.fence = Some(fence);
        } else if trimmed.is_empty() || thematic_break(trimmed) {
            self.close();
        } else if trimmed.starts_with('|') {
            self.interrupt(indent);
        } else if markdown::list_item(trimmed) {
            self.item(number, indent, trimmed);
        } else {
            self.text(number, indent, trimmed);
        }
    }

    /// Code or a table ends the candidate being read; at the margin it also
    /// ends the list.
    fn interrupt(&mut self, indent: usize) {
        self.close();
        self.settle(None);
        if indent == 0 {
            self.end_list();
        }
    }

    fn end_list(&mut self) {
        self.items.clear();
        self.introduction = None;
    }

    /// A list item starts: its parent is the nearest item indented less.
    fn item(&mut self, number: usize, indent: usize, trimmed: &str) {
        self.close();
        self.settle(Some(indent));
        self.items.retain(|(at, _)| *at < indent);
        let lead_in = match self.items.last() {
            Some((_, parent)) => Some(parent.clone()),
            None => self.introduction.clone(),
        };
        self.start(
            number,
            lead_in,
            without_marker(trimmed),
            Block::Item(indent),
        );
    }

    /// A line of text continues the candidate being read, or starts a
    /// paragraph; one at the margin after a blank line ends the list. A
    /// quote (`>`) starts a paragraph of its own.
    fn text(&mut self, number: usize, indent: usize, trimmed: &str) {
        let block = if trimmed.starts_with('>') {
            Block::Quote
        } else {
            Block::Paragraph
        };
        let text = trimmed.trim_start_matches('>').trim();
        let continued = self
            .open
            .as_mut()
            .filter(|open| block == Block::Paragraph || open.block == Block::Quote);
        if let Some(open) = continued {
            open.line.text.push(' ');
            open.line.text.push_str(text);
            open.line.end_line = number;
            return;
        }
        self.close();
        self.settle(None);
        if indent == 0 {
            self.end_list();
        }
        let lead_in = self.items.last().map(|(_, parent)| parent.clone());
        self.start(number, lead_in, text, block);
    }

    fn start(&mut self, number: usize, lead_in: Option<String>, text: &str, block: Block) {
        self.open = Some(Open {
            line: Line {
                start_line: number,
                end_line: number,
                heading: self.heading.to_string(),
                lead_in,
                text: text.to_string(),
            },
            block,
        });
    }

    /// Finish the candidate being read: an item may enclose the next ones,
    /// and one ending in `:` waits to see whether it introduces a list.
    fn close(&mut self) {
        let Some(mut open) = self.open.take() else {
            return;
        };
        open.line.text = clean(&open.line.text);
        if let Block::Item(indent) = open.block {
            self.items.push((indent, open.line.text.clone()));
        }
        if open.line.text.ends_with(':') {
            self.settle(None);
            self.held = Some(open);
        } else {
            self.push(open.line);
        }
    }

    /// Decide a held candidate by what follows it: a list right after a
    /// paragraph, or items nested under an item, make it their lead-in and
    /// no rule of its own; anything else makes it a candidate.
    fn settle(&mut self, next_item: Option<usize>) {
        let Some(held) = self.held.take() else {
            return;
        };
        match (held.block, next_item) {
            (Block::Item(parent), Some(child)) if child > parent => {}
            (Block::Paragraph | Block::Quote, Some(_)) => {
                self.introduction = Some(held.line.text);
            }
            _ => self.push(held.line),
        }
    }

    fn push(&mut self, line: Line) {
        if worth_asking(&line.text) {
            self.found.push(line);
        }
    }

    fn finish(mut self) -> Vec<Line> {
        self.close();
        self.settle(None);
        self.found
    }
}

/// Leading whitespace, a tab counting as [`TAB_WIDTH`] spaces.
fn indentation(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

/// A thematic break or a setext heading's underline: `---`, `* * *`, `===`.
fn thematic_break(trimmed: &str) -> bool {
    let marks: Vec<char> = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3
        && ['-', '*', '_', '=']
            .iter()
            .any(|mark| marks.iter().all(|c| c == mark))
}

/// A list item's text after its marker (`-`, `*`, `+`, `1.`) and a task box.
fn without_marker(item: &str) -> &str {
    let digits = item.chars().take_while(char::is_ascii_digit).count();
    // `markdown::list_item` found one marker character, or digits and a dot.
    let marker = if digits > 0 { digits + 1 } else { 1 };
    let rest = item[marker..].trim_start();
    ["[ ] ", "[x] ", "[X] "]
        .iter()
        .find_map(|box_| rest.strip_prefix(box_))
        .unwrap_or(rest)
}

/// Words joined by single spaces, of [`printable`] characters only.
fn clean(text: &str) -> String {
    text.split_whitespace()
        .map(|word| word.chars().filter(|c| printable(*c)).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a candidate could state a rule: it has words, is no `@path`
/// import or bare HTML tag, and is not a page long.
fn worth_asking(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
        && !text.split_whitespace().all(|word| word.starts_with('@'))
        && !(text.starts_with('<') && text.ends_with('>'))
        && text.chars().count() <= MAX_CHARS
}
