//! Top-level statement blocks of a function body: the places a reader could
//! extract into a named function. They locate a finding; they never decide one.
use super::{is_comment, line_of};
use std::ops::Range;
use tree_sitter::Node;

/// At most this many blocks are offered; adjacent short blocks merge first.
pub const MAX_BLOCKS: usize = 12;
/// A statement this long ends its block, so a loop or branch stays with the
/// short setup above it.
const LONG_STATEMENT_LINES: usize = 3;

const BODY_KINDS: [&str; 6] = [
    "block",
    "statement_block",
    "compound_statement",
    "body_statement",
    "block_body",
    "constructor_body",
];

/// Byte ranges of the body's blocks, in order, with leading comments attached.
/// A statement that wraps most of the body (a `with`, loop or `try` around the
/// work) is read through: its inner statements become the candidates. Fewer
/// than two blocks offer no choice.
pub fn blocks(body: Node<'_>, source: &str) -> Vec<Range<usize>> {
    if !BODY_KINDS.contains(&body.kind()) {
        return Vec::new();
    }
    let mut statements = statements(body);
    while let Some((at, inner)) = wrapper(&statements, source) {
        statements.splice(at..=at, self::statements(inner));
    }
    let mut blocks = group(&statements, source);
    while blocks.len() > MAX_BLOCKS {
        merge_smallest(&mut blocks, source);
    }
    if blocks.len() < 2 {
        return Vec::new();
    }
    blocks
}

/// The statement with an inner body that spans more lines than every other
/// statement together, and that inner body.
fn wrapper<'a>(statements: &[(usize, Node<'a>)], source: &str) -> Option<(usize, Node<'a>)> {
    let lines =
        |node: Node<'_>| line_of(source, node.end_byte()) + 1 - line_of(source, node.start_byte());
    let total: usize = statements.iter().map(|(_, node)| lines(*node)).sum();
    statements.iter().enumerate().find_map(|(at, (_, node))| {
        let inner = inner_body(*node)?;
        (2 * lines(*node) > total && inner.named_child_count() > 0).then_some((at, inner))
    })
}

/// Statements with the start of the comments directly above them. A Go
/// block holds its statements in one `statement_list`.
fn statements(body: Node<'_>) -> Vec<(usize, Node<'_>)> {
    let body = match body.named_child(0) {
        Some(list) if body.named_child_count() == 1 && list.kind() == "statement_list" => list,
        _ => body,
    };
    let mut cursor = body.walk();
    let mut found = Vec::new();
    let mut comment_start = None;
    for child in body.named_children(&mut cursor) {
        if is_comment(child) {
            comment_start.get_or_insert(child.start_byte());
        } else {
            found.push((comment_start.take().unwrap_or(child.start_byte()), child));
        }
    }
    found
}

fn inner_body(statement: Node<'_>) -> Option<Node<'_>> {
    let node = if statement.kind() == "expression_statement" {
        statement.named_child(0)?
    } else {
        statement
    };
    // A Ruby call wraps its work in a block: `File.open(path) do |file| … end`.
    let node = node.child_by_field_name("block").unwrap_or(node);
    node.child_by_field_name("body")
        .filter(|body| BODY_KINDS.contains(&body.kind()))
}

/// A block runs until a blank line or through the first long statement.
fn group(statements: &[(usize, Node<'_>)], source: &str) -> Vec<Range<usize>> {
    let mut blocks: Vec<Range<usize>> = Vec::new();
    let mut open = false;
    let mut previous_end = None;
    for &(start, node) in statements {
        let long = line_of(source, node.end_byte()) + 1 - line_of(source, node.start_byte())
            >= LONG_STATEMENT_LINES;
        let blank_line = previous_end.is_some_and(|end: usize| {
            let lines: Vec<&str> = source[end..start].split('\n').collect();
            lines.len() > 2
                && lines[1..lines.len() - 1]
                    .iter()
                    .any(|l| l.trim().is_empty())
        });
        match blocks.last_mut() {
            Some(block) if open && !blank_line => block.end = node.end_byte(),
            _ => blocks.push(start..node.end_byte()),
        }
        open = !long;
        previous_end = Some(node.end_byte());
    }
    blocks
}

fn merge_smallest(blocks: &mut Vec<Range<usize>>, source: &str) {
    let lines = |range: &Range<usize>| line_of(source, range.end) - line_of(source, range.start);
    let Some(at) =
        (0..blocks.len() - 1).min_by_key(|&i| lines(&(blocks[i].start..blocks[i + 1].end)))
    else {
        return;
    };
    let next = blocks.remove(at + 1);
    blocks[at].end = next.end;
}

#[cfg(test)]
mod tests {
    use crate::analysis::units::parse;
    use std::path::Path;

    fn block_lines(path: &str, source: &str) -> Vec<String> {
        let parsed = parse(Path::new(path), source).unwrap();
        parsed.units[0]
            .blocks
            .iter()
            .map(|b| source[b.clone()].lines().next().unwrap().trim().to_string())
            .collect()
    }

    #[test]
    fn blocks_end_at_blank_lines_and_after_long_statements() {
        let source = "fn run(items: &[i32]) -> i32 {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n\n    // Sum the positive items.\n    let mut total = 0;\n    for item in items {\n        if *item > 0 {\n            total += item;\n        }\n    }\n    total + a + b + c\n}\n";
        assert_eq!(
            block_lines("lib.rs", source),
            [
                "let a = 1;",
                "// Sum the positive items.",
                "total + a + b + c"
            ]
        );
    }

    #[test]
    fn a_body_wrapped_in_one_statement_is_read_through_it() {
        let source = "def load(path):\n    with open(path) as f:\n        header = f.readline()\n\n        rows = [line.split(',') for line in f]\n        return header, rows\n";
        assert_eq!(
            block_lines("load.py", source),
            [
                "header = f.readline()",
                "rows = [line.split(',') for line in f]"
            ]
        );
    }

    #[test]
    fn a_statement_wrapping_most_of_the_body_is_read_through() {
        let source = "def render():\n    prepare()\n    with open('out') as out:\n        header = build()\n        out.write(header)\n\n        for row in rows:\n            out.write(row)\n            out.flush()\n    print('done')\n";
        assert_eq!(
            block_lines("render.py", source),
            ["prepare()", "for row in rows:", "print('done')"]
        );
    }

    #[test]
    fn a_single_block_offers_no_choice() {
        let source = "function f(a) {\n  const b = a + 1\n  return b\n}\n";
        assert!(block_lines("f.js", source).is_empty());
    }
}
