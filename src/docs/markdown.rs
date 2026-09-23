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

/// The top-level blocks of a section, each a paragraph or a list item with
/// its nested lines and code, carrying the section's heading.
pub fn blocks(source: &str, section: &Section) -> Vec<Section> {
    let lines: Vec<&str> = source.lines().collect();
    let first = section.start_line - 1 + usize::from(!section.heading.is_empty());
    let last = section.end_line.min(lines.len());
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

/// A list item marker at the start of a line: `-`, `*`, `+` or `1.`.
fn list_item(line: &str) -> bool {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    ["- ", "* ", "+ "].iter().any(|m| line.starts_with(m))
        || (digits > 0 && line[digits..].starts_with(". "))
}

/// An ATX heading's text: one to six `#` then a space.
fn heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    let rest = &trimmed[hashes..];
    ((1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with([' ', '\t'])))
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
    fn sections_split_on_headings_outside_code() {
        let source = "---\nalwaysApply: true\nglobs: [\"src/**\", 'web/**']\n---\nIntro line.\n\n# Build\nRun `make`.\n```sh\n# not a heading\n```\n## Empty\n### Test\n<!-- hidden -->\nRun tests.\n";
        let doc = parse(source);
        assert_eq!(doc.frontmatter["alwaysApply"], "true");
        assert_eq!(doc.frontmatter["globs"], "src/**,web/**");
        let headings: Vec<&str> = doc.sections.iter().map(|s| s.heading.as_str()).collect();
        assert_eq!(headings, ["", "Build", "Test"]);
        assert_eq!(doc.sections[0].start_line, 5);
        assert!(doc.sections[1].text.contains("# not a heading"));
        assert_eq!(doc.sections[1].start_line, 7);
        assert_eq!(doc.sections[1].end_line, 11);
        assert_eq!(doc.sections[2].text, "Run tests.");
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
    fn frontmatter_lists_and_imports() {
        let doc = parse("---\npaths:\n  - \"src/**\"\n  - tests/**\n---\n# A\nx\n");
        assert_eq!(doc.frontmatter["paths"], "src/**,tests/**");
        let found = imports(
            "See @AGENTS.md and @docs/git.md.\nEmail a@b.c, `npm i @types/node`, @posthog/browser\n```\n@skip.md\n```\n",
        );
        assert_eq!(
            found,
            [(1, "AGENTS.md".to_string()), (1, "docs/git.md".to_string())]
        );
    }

    #[test]
    fn comments_are_stripped_and_tokens_estimated() {
        assert_eq!(strip_comments("a<!-- b -->c<!-- d"), "ac");
        assert_eq!(tokens("abcdefgh"), 2);
        assert_eq!(tokens("abcdefghi"), 3);
    }
}
