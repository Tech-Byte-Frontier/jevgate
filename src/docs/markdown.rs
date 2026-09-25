//! Markdown as the documentation rules see it: frontmatter keys, heading
//! sections with their lines, `@path` imports, and the text a harness keeps
//! after dropping HTML comments.
use std::collections::BTreeMap;

/// Text under one heading, up to the next heading of any level.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Empty for the text before the first heading.
    pub heading: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Markdown {
    /// Keys of a leading `---` block; list values are joined with commas.
    pub frontmatter: BTreeMap<String, String>,
    pub sections: Vec<Section>,
}

pub fn parse(source: &str) -> Markdown {
    let lines: Vec<&str> = source.lines().collect();
    let (frontmatter, body) = frontmatter(&lines);
    let mut sections = Vec::new();
    let mut current = Section {
        heading: String::new(),
        start_line: body + 1,
        end_line: body + 1,
        text: String::new(),
    };
    let mut fence: Option<&str> = None;
    let mut text = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(body) {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
        } else if let Some(open) = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f)) {
            fence = Some(open);
        } else if let Some(heading) = heading(line) {
            finish(&mut sections, current, &mut text, index);
            current = Section {
                heading,
                start_line: index + 1,
                end_line: index + 1,
                text: String::new(),
            };
            continue;
        }
        text.push(*line);
    }
    finish(&mut sections, current, &mut text, lines.len());
    Markdown {
        frontmatter,
        sections,
    }
}

/// Close a section at `end` (exclusive, zero-based); empty sections are dropped.
fn finish(sections: &mut Vec<Section>, mut section: Section, text: &mut Vec<&str>, end: usize) {
    let body = strip_comments(&text.join("\n")).trim().to_string();
    text.clear();
    if body.is_empty() {
        return;
    }
    section.end_line = end.max(section.start_line);
    section.text = body;
    sections.push(section);
}

/// A heading outside code blocks: its one-based line, level and text.
#[derive(Clone, Debug, PartialEq)]
pub struct Heading {
    pub line: usize,
    pub level: usize,
    pub text: String,
}

/// Every ATX heading outside fenced code, in order.
pub fn headings(source: &str) -> Vec<Heading> {
    let lines: Vec<&str> = source.lines().collect();
    let (_, body) = frontmatter(&lines);
    let mut fence: Option<&str> = None;
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(body) {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
        } else if let Some(open) = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f)) {
            fence = Some(open);
        } else if let Some(text) = heading(line) {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            found.push(Heading {
                line: index + 1,
                level,
                text,
            });
        }
    }
    found
}

/// The top-level blocks of a section, each a paragraph or a list item with
/// its nested lines and code, carrying the section's heading.
pub fn blocks(source: &str, section: &Section) -> Vec<Section> {
    let lines: Vec<&str> = source.lines().collect();
    let first = section.start_line - 1 + usize::from(!section.heading.is_empty());
    let last = section.end_line.min(lines.len());
    let starts = block_starts(&lines, first, last);
    let mut blocks = Vec::new();
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(last);
        let text = strip_comments(&lines[start..end].join("\n"))
            .trim()
            .to_string();
        if !text.is_empty() {
            let used = lines[start..end]
                .iter()
                .rposition(|l| !l.trim().is_empty())
                .unwrap_or(0);
            blocks.push(Section {
                heading: section.heading.clone(),
                start_line: start + 1,
                end_line: start + used + 1,
                text,
            });
        }
    }
    blocks
}

/// Indexes of the lines in `first..last` that start a block: an unindented
/// line after a blank one, or an unindented list item, outside code fences.
fn block_starts(lines: &[&str], first: usize, last: usize) -> Vec<usize> {
    let mut starts = Vec::new();
    let (mut fence, mut blank) = (false, true);
    for (index, line) in lines.iter().enumerate().take(last).skip(first) {
        let fenced_line =
            line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~");
        if !fence
            && !line.trim().is_empty()
            && (blank || list_item(line))
            && !line.starts_with([' ', '\t'])
        {
            starts.push(index);
        }
        if fenced_line {
            fence = !fence;
        }
        blank = line.trim().is_empty();
    }
    starts
}

/// A list item marker at the start of a line: `-`, `*`, `+` or `1.`.
fn list_item(line: &str) -> bool {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    ["- ", "* ", "+ "].iter().any(|m| line.starts_with(m))
        || (digits > 0 && line[digits..].starts_with(". "))
}

/// Markdown's deepest heading level.
pub const DEEPEST_HEADING: usize = 6;
/// More leading spaces than this make a line code, not a heading.
const HEADING_INDENT: usize = 3;

/// An ATX heading's text: one to six `#` then a space.
fn heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > HEADING_INDENT {
        return None;
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    let rest = &trimmed[hashes..];
    ((1..=DEEPEST_HEADING).contains(&hashes) && (rest.is_empty() || rest.starts_with([' ', '\t'])))
        .then(|| rest.trim().trim_end_matches('#').trim().to_string())
}

/// The frontmatter keys and the zero-based line where the body starts.
fn frontmatter(lines: &[&str]) -> (BTreeMap<String, String>, usize) {
    let mut keys = BTreeMap::new();
    if lines.first().map(|l| l.trim()) != Some("---") {
        return (keys, 0);
    }
    let Some(close) = lines.iter().skip(1).position(|l| l.trim() == "---") else {
        return (keys, 0);
    };
    let mut last: Option<String> = None;
    for line in &lines[1..=close] {
        if let Some(item) = line.trim().strip_prefix("- ")
            && let Some(key) = &last
        {
            let value: &mut String = keys.entry(key.clone()).or_default();
            if !value.is_empty() {
                value.push(',');
            }
            value.push_str(unquote(item));
        } else if let Some((key, value)) = line.split_once(':')
            && !line.starts_with([' ', '\t'])
        {
            let value = value.trim().trim_start_matches('[').trim_end_matches(']');
            let items: Vec<&str> = value.split(',').map(unquote).collect();
            keys.insert(key.trim().to_string(), items.join(","));
            last = Some(key.trim().to_string());
        }
    }
    (keys, close + 2)
}

fn unquote(value: &str) -> &str {
    value.trim().trim_matches(['"', '\''])
}

/// The text without HTML comments, which Claude Code drops before loading.
pub fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// `@path` imports outside code, with their one-based lines.
pub fn imports(source: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut fenced = false;
    for (index, line) in strip_comments(source).lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // Text inside inline code spans is not an import.
        let prose: String = line
            .split('`')
            .enumerate()
            .filter(|(i, _)| i % 2 == 0)
            .map(|(_, part)| part)
            .collect::<Vec<_>>()
            .join(" ");
        for word in prose.split_whitespace() {
            let Some(path) = word.strip_prefix('@') else {
                continue;
            };
            let path = path.trim_end_matches([',', '.', ';', ':', ')']);
            // A file path: an extension on the last part, or a relative or
            // home prefix. Scoped package names such as `@types/node` are not.
            let last = path.rsplit('/').next().unwrap_or(path);
            let file = last.contains('.') && !last.ends_with('.');
            let relative = ["./", "../", "~/"].iter().any(|p| path.starts_with(p));
            if (file || relative) && !path.contains('@') {
                found.push((index + 1, path.to_string()));
            }
        }
    }
    found
}

/// Estimated tokens a harness spends on `text`, at four bytes per token.
pub fn tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_keys_join_inline_and_dash_lists() {
        let inline = parse("---\nalwaysApply: true\nglobs: [\"src/**\", 'web/**']\n---\n# A\nx\n");
        assert_eq!(inline.frontmatter["alwaysApply"], "true");
        assert_eq!(inline.frontmatter["globs"], "src/**,web/**");
        let dashed = parse("---\npaths:\n  - \"src/**\"\n  - tests/**\n---\n# A\nx\n");
        assert_eq!(dashed.frontmatter["paths"], "src/**,tests/**");
    }

    #[test]
    fn sections_split_on_headings_outside_code_and_drop_empty_ones() {
        let source = "Intro line.\n\n# Build\nRun `make`.\n```sh\n# not a heading\n```\n## Empty\n### Test\nRun tests.\n";
        let doc = parse(source);
        let spans: Vec<(&str, usize, usize)> = doc
            .sections
            .iter()
            .map(|s| (s.heading.as_str(), s.start_line, s.end_line))
            .collect();
        assert_eq!(spans, [("", 1, 2), ("Build", 3, 7), ("Test", 9, 10)]);
    }

    #[test]
    fn section_text_leaves_out_html_comments() {
        let doc = parse("# Test\n<!-- note for people -->\nRun tests.\n");
        assert_eq!(doc.sections[0].text, "Run tests.");
    }

    #[test]
    fn headings_skip_code_and_keep_levels() {
        let found = headings("# A\n```\n# no\n```\n### B ###\n");
        assert_eq!(
            found,
            [
                Heading {
                    line: 1,
                    level: 1,
                    text: "A".into()
                },
                Heading {
                    line: 5,
                    level: 3,
                    text: "B".into()
                },
            ]
        );
    }

    #[test]
    fn blocks_are_paragraphs_and_top_level_list_items() {
        let source = "# Rules\n- One\n  continued\n- Two\n  ```sh\n- not an item\n  ```\n\nA paragraph\nwrapped.\n1. Numbered\n";
        let doc = parse(source);
        let blocks = blocks(source, &doc.sections[0]);
        let spans: Vec<(usize, usize)> =
            blocks.iter().map(|b| (b.start_line, b.end_line)).collect();
        assert_eq!(spans, [(2, 3), (4, 7), (9, 10), (11, 11)]);
        assert_eq!(blocks[2].text, "A paragraph\nwrapped.");
        assert!(blocks.iter().all(|b| b.heading == "Rules"));
    }

    #[test]
    fn imports_are_file_paths_outside_code() {
        let found = imports(
            "See @AGENTS.md and @docs/git.md.\nEmail a@b.c, `npm i @types/node`, @posthog/browser\n```\n@skip.md\n```\n",
        );
        assert_eq!(
            found,
            [(1, "AGENTS.md".to_string()), (1, "docs/git.md".to_string())]
        );
    }

    #[test]
    fn an_unclosed_comment_drops_the_rest() {
        assert_eq!(strip_comments("a<!-- b -->c<!-- d"), "ac");
    }

    #[test]
    fn tokens_round_up_at_four_bytes_each() {
        assert_eq!((tokens("abcdefgh"), tokens("abcdefghi")), (2, 3));
    }
}
