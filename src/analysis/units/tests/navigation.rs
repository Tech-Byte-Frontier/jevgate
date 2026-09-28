//! The generic tier against GitHub's code navigation: every definition a
//! grammar's own `tags.scm` finds is a unit or owns one.
use super::generic::{C, CPP, DART, ELIXIR, LUA, SWIFT};
use super::*;

/// The function, method and type names a grammar's own `tags.scm` finds in
/// `source`, as GitHub's code navigation shows them: C and C++ tag the
/// declarator of a prototype as they tag a definition's, so a declarator
/// outside every function definition is left out.
fn navigation_names(language: tree_sitter::Language, tags: &str, source: &str) -> Vec<String> {
    use tree_sitter::StreamingIterator;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let query = tree_sitter::Query::new(&language, tags).unwrap();
    let captures = query.capture_names();
    let definitions = ["function", "method", "class", "interface", "module", "type"];
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    let mut names = Vec::new();
    while let Some(found) = matches.next() {
        let kind = |c: &tree_sitter::QueryCapture<'_>| captures[c.index as usize];
        let defined = found.captures().iter().find(|c| {
            kind(c)
                .strip_prefix("definition.")
                .is_some_and(|d| definitions.contains(&d))
        });
        let name = found.captures().iter().find(|c| kind(c) == "name");
        let (Some(defined), Some(name)) = (defined, name) else {
            continue;
        };
        let prototype = defined.node.kind() == "function_declarator"
            && std::iter::successors(defined.node.parent(), |n| n.parent())
                .all(|n| n.kind() != "function_definition");
        if !prototype {
            names.push(name.node.utf8_text(source.as_bytes()).unwrap().to_string());
        }
    }
    names
}

#[test]
fn every_definition_github_s_navigation_tags_is_a_unit_or_owns_one() {
    let samples = [
        (
            "editor.c",
            C,
            tree_sitter_c::LANGUAGE,
            tree_sitter_c::TAGS_QUERY,
        ),
        (
            "cart.cpp",
            CPP,
            tree_sitter_cpp::LANGUAGE,
            tree_sitter_cpp::TAGS_QUERY,
        ),
        (
            "Shop.swift",
            SWIFT,
            tree_sitter_swift::LANGUAGE,
            tree_sitter_swift::TAGS_QUERY,
        ),
        (
            "counter.dart",
            DART,
            tree_sitter_dart::LANGUAGE,
            tree_sitter_dart::TAGS_QUERY,
        ),
        (
            "cart.ex",
            ELIXIR,
            tree_sitter_elixir::LANGUAGE,
            tree_sitter_elixir::TAGS_QUERY,
        ),
        (
            "util.lua",
            LUA,
            tree_sitter_lua::LANGUAGE,
            tree_sitter_lua::TAGS_QUERY,
        ),
    ];
    for (path, source, language, tags) in samples {
        let units = parse(Path::new(path), source).unwrap().units;
        let tagged = navigation_names(language.into(), tags, source);
        assert!(!tagged.is_empty(), "{path}");
        for name in tagged {
            assert!(
                units
                    .iter()
                    .any(|u| u.short_name == name || u.owner == name),
                "{path}: {name}"
            );
        }
    }
}
