//! The comments and string literals of a parsed file: text written for
//! people, or data, rather than code. Steering reads their text; the guards
//! ask which of them a marker sits in, since `# noqa` quoted in a string turns
//! nothing off and a `.skip(` in a comment skips no test.
use super::comments;
use anyhow::Result;
use std::{ops::Range, path::Path};
use tree_sitter::Node;

/// String literals in the grammars of the languages with analyzers of
/// their own; the generic tier's table names its languages'
/// (`generic::Language::strings`). A string's parts are not visited apart
/// from it.
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
        let kinds = crate::analysis::generic::read(path, source)
            .map_or(STRING_KINDS, |language| language.strings);
        let mut strings = Vec::new();
        collect_strings(root, kinds, &mut strings);
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

/// String literal nodes of `kinds`, outermost only.
fn collect_strings(node: Node<'_>, kinds: &[&str], found: &mut Vec<Range<usize>>) {
    if kinds.contains(&node.kind()) {
        found.push(node.byte_range());
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_strings(child, kinds, found);
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

    #[test]
    fn the_generic_tier_s_strings_are_strings() {
        for (path, source) in [
            ("View.swift", "let a = \"# noqa\"\n"),
            ("View.swift", "let a = \"\"\"\n# noqa\n\"\"\"\n"),
            ("run.sh", "a='# noqa'\n"),
            ("run.sh", "a=\"# noqa\"\n"),
            ("run.sh", "cat <<EOF\n# noqa\nEOF\n"),
            ("Shop.kt", "val a = \"\"\"\n# noqa\n\"\"\"\n"),
            ("cart.cpp", "auto a = R\"(# noqa)\";\n"),
            ("lib.ex", "a = ~s(# noqa)\n"),
        ] {
            let regions = Regions::of(Path::new(path), source).unwrap().unwrap();
            let at = source.find("# noqa").unwrap();
            assert_eq!(regions.at(at), Region::String, "{path}: {source}");
        }
    }
}
