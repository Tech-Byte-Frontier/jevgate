//! The comments and string literals of a parsed file: text written for
//! people, or data, rather than code. Steering reads their text; the guards
//! ask which of them a marker sits in, since `# noqa` quoted in a string turns
//! nothing off and a `.skip(` in a comment skips no test.
use super::comments;
use anyhow::Result;
use std::{ops::Range, path::Path};
use tree_sitter::Node;

/// String literals in the grammars JevGate parses; a string's parts are not
/// visited apart from it.
const STRING_KINDS: &[&str] = &[
    "string",
    "string_literal",
    "interpreted_string_literal",
    "raw_string_literal",
    "encapsed_string",
    "template_string",
    "verbatim_string_literal",
    "interpolated_string_expression",
];

/// What a byte of a file is part of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Comment,
    String,
    Code,
}

/// A parsed file's comments (consecutive line comments merged, docstrings
/// included) and outermost string literals, as byte spans in order.
pub struct Regions {
    pub comments: Vec<Range<usize>>,
    pub strings: Vec<Range<usize>>,
}

impl Regions {
    /// The regions of `source`, the file at `path`; none when no grammar
    /// reads its language.
    pub fn of(path: &Path, source: &str) -> Result<Option<Self>> {
        let Some(tree) = crate::syntax::parse(path, source)? else {
            return Ok(None);
        };
        let root = tree.root_node();
        let mut strings = Vec::new();
        collect_strings(root, &mut strings);
        Ok(Some(Self {
            comments: comments::spans(path, root, source),
            strings,
        }))
    }

    /// What the byte at `at` is part of; a docstring is a comment.
    pub fn at(&self, at: usize) -> Region {
        let inside = |spans: &[Range<usize>]| spans.iter().any(|span| span.contains(&at));
        if inside(&self.comments) {
            Region::Comment
        } else if inside(&self.strings) {
            Region::String
        } else {
            Region::Code
        }
    }
}

/// String literal nodes, outermost only.
fn collect_strings(node: Node<'_>, found: &mut Vec<Range<usize>>) {
    if STRING_KINDS.contains(&node.kind()) {
        found.push(node.byte_range());
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_strings(child, found);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_strings_and_code_are_told_apart() {
        let source = "# noqa in a comment\nrule = \"# noqa\"\n@pytest.mark.skip\ndef test_a():\n    \"\"\"Docs.\"\"\"\n";
        let regions = Regions::of(Path::new("tests/test_a.py"), source)
            .unwrap()
            .unwrap();
        let at = |needle: &str, nth: usize| source.match_indices(needle).nth(nth).unwrap().0;
        assert_eq!(regions.at(at("noqa", 0)), Region::Comment);
        assert_eq!(regions.at(at("noqa", 1)), Region::String);
        assert_eq!(regions.at(at("@pytest", 0)), Region::Code);
        assert_eq!(regions.at(at("Docs", 0)), Region::Comment, "a docstring");
        assert!(
            Regions::of(Path::new("notes.txt"), "text")
                .unwrap()
                .is_none()
        );
    }
}
