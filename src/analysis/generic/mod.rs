//! The generic tier: languages read through a tree-sitter tag query instead
//! of a hand-written analyzer. One table entry per language names its
//! grammar, extensions and query, the node kinds the shared analyses need
//! (statement blocks, control flow, literals) and where it keeps its tests;
//! `analysis::units::generic` turns the query's captures into units. Only
//! the rules those units can serve judge these files: function
//! simplification, file organization, shared logic and comments. Every
//! language here is in preview: it becomes supported once two projects
//! JevGate was never tuned on meet the maturity bar (a rule and level right
//! at least 80% of the time over at least 20 labeled findings).
//!
//! The queries are JevGate's own, written in the captures GitHub's code
//! navigation uses (`@definition.function`, `@definition.class`, `@name`,
//! `@reference.call`), with the captured node the whole definition. The
//! grammars' own `tags.scm` capture what names a definition, not the
//! definition: C tags the declarator of a prototype and a definition alike,
//! Swift tags a method as its whole class and Dart its signature without its
//! body, while Kotlin and Bash ship none and Scala does not export its.
mod tags;

use std::{path::Path, sync::OnceLock};
pub(crate) use tags::{Defines, Tag, tags};
use tree_sitter::Query;

/// A language of the generic tier.
pub(crate) struct Language {
    /// What reports and requests call it.
    pub name: &'static str,
    /// Languages whose code can be shared between their files: C and C++.
    pub family: &'static str,
    extensions: &'static [&'static str],
    grammar: fn() -> tree_sitter::Language,
    /// The tag query: definitions (`@definition.function`, `.method`,
    /// `.class`, `.interface`, `.module`, `.object`, `.type`) with their
    /// `@name`, an optional `@body` where the grammar has no `body` field and
    /// an optional `@scope` a definition is written in (`Cart::add`), and
    /// calls (`@reference.call` with their `@name`).
    tags: &'static str,
    /// Kinds whose named children are statements: a body and the blocks in it.
    pub blocks: &'static [&'static str],
    /// Kinds that nest control flow.
    pub control: &'static [&'static str],
    /// Conditionals: one in another's `else` continues its chain rather
    /// than nesting.
    pub conditionals: &'static [&'static str],
    /// Clauses a conditional lists: `elif`, `elseif` and `else`, or C's
    /// `else` holding the next conditional.
    pub clauses: &'static [&'static str],
    /// Leaves that hold literal values, which copies may differ in.
    pub literals: &'static [&'static str],
    tests: Tests,
    query: OnceLock<Query>,
}

/// Where a language keeps its tests, beyond the conventions every language
/// shares (`test/`, `tests/`, `test_*`, `*_test.*`, `*.spec.*`).
struct Tests {
    /// File stems' endings, by case: `OrdersTest`, not `Contest`.
    stems: &'static [&'static str],
    /// File name endings, in lower case: `_spec.lua`, `.bats`.
    names: &'static [&'static str],
    /// Directory names, in lower case: busted's `spec`, Dart's `integration_test`.
    directories: &'static [&'static str],
    /// Directory names' endings, by case: a Kotlin source set such as
    /// `androidTest`, a Swift test target such as `VaporTests`.
    directory_ends: &'static [&'static str],
}

impl Tests {
    const NONE: Tests = Tests {
        stems: &[],
        names: &[],
        directories: &[],
        directory_ends: &[],
    };
}

/// Test classes named as JUnit, Kotest, ScalaTest and XCTest name them.
const CLASS_TESTS: &[&str] = &["Test", "Tests", "Spec", "IT"];

const C_LITERALS: &[&str] = &["string_content", "number_literal", "char_literal"];
const C_CONTROL: &[&str] = &[
    "if_statement",
    "for_statement",
    "while_statement",
    "do_statement",
    "switch_statement",
    "conditional_expression",
];

static LANGUAGES: [Language; 9] = [
    Language {
        name: "C",
        family: "C",
        extensions: &["c", "h"],
        grammar: || tree_sitter_c::LANGUAGE.into(),
        tags: include_str!("queries/c.scm"),
        blocks: &["compound_statement"],
        control: C_CONTROL,
        conditionals: &["if_statement"],
        clauses: &["else_clause"],
        literals: C_LITERALS,
        tests: Tests::NONE,
        query: OnceLock::new(),
    },
    Language {
        name: "C++",
        family: "C",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        grammar: || tree_sitter_cpp::LANGUAGE.into(),
        tags: include_str!("queries/cpp.scm"),
        blocks: &["compound_statement"],
        control: &[
            "if_statement",
            "for_statement",
            "for_range_loop",
            "while_statement",
            "do_statement",
            "switch_statement",
            "try_statement",
            "conditional_expression",
        ],
        conditionals: &["if_statement"],
        clauses: &["else_clause"],
        literals: &[
            "string_content",
            "raw_string_content",
            "number_literal",
            "char_literal",
        ],
        tests: Tests {
            stems: &["Test", "Tests"],
            names: &["_unittest.cc", "_unittest.cpp"],
            directories: &[],
            directory_ends: &[],
        },
        query: OnceLock::new(),
    },
    Language {
        name: "Kotlin",
        family: "Kotlin",
        extensions: &["kt", "kts"],
        grammar: || tree_sitter_kotlin_ng::LANGUAGE.into(),
        tags: include_str!("queries/kotlin.scm"),
        blocks: &["block", "lambda_literal"],
        control: &[
            "if_expression",
            "when_expression",
            "for_statement",
            "while_statement",
            "do_while_statement",
            "try_expression",
        ],
        conditionals: &["if_expression"],
        clauses: &[],
        literals: &[
            "string_content",
            "number_literal",
            "float_literal",
            "character_literal",
        ],
        tests: Tests {
            stems: CLASS_TESTS,
            names: &[],
            directories: &[],
            directory_ends: &["Test"],
        },
        query: OnceLock::new(),
    },
    Language {
        name: "Swift",
        family: "Swift",
        extensions: &["swift"],
        grammar: || tree_sitter_swift::LANGUAGE.into(),
        tags: include_str!("queries/swift.scm"),
        blocks: &["statements"],
        control: &[
            "if_statement",
            "for_statement",
            "while_statement",
            "repeat_while_statement",
            "switch_statement",
            "do_statement",
            "ternary_expression",
        ],
        conditionals: &["if_statement"],
        clauses: &[],
        literals: &[
            "line_str_text",
            "multi_line_str_text",
            "integer_literal",
            "real_literal",
            "hex_literal",
            "bin_literal",
            "oct_literal",
        ],
        tests: Tests {
            stems: CLASS_TESTS,
            names: &[],
            directories: &[],
            directory_ends: &["Tests"],
        },
        query: OnceLock::new(),
    },
    Language {
        name: "Bash",
        family: "Bash",
        extensions: &["sh", "bash", "bats"],
        grammar: || tree_sitter_bash::LANGUAGE.into(),
        tags: include_str!("queries/bash.scm"),
        blocks: &["compound_statement", "do_group"],
        control: &[
            "if_statement",
            "for_statement",
            "c_style_for_statement",
            "while_statement",
            "case_statement",
        ],
        conditionals: &["if_statement"],
        clauses: &["elif_clause", "else_clause"],
        literals: &["string_content", "raw_string", "ansi_c_string", "number"],
        tests: Tests {
            stems: &[],
            names: &[".bats"],
            directories: &[],
            directory_ends: &[],
        },
        query: OnceLock::new(),
    },
    Language {
        name: "Dart",
        family: "Dart",
        extensions: &["dart"],
        grammar: || tree_sitter_dart::LANGUAGE.into(),
        tags: include_str!("queries/dart.scm"),
        blocks: &["block"],
        control: &[
            "if_statement",
            "for_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "switch_expression",
            "try_statement",
            "conditional_expression",
        ],
        conditionals: &["if_statement"],
        clauses: &[],
        literals: &[
            "template_chars_single",
            "template_chars_double",
            "template_chars_single_single",
            "template_chars_double_single",
            "decimal_integer_literal",
            "decimal_floating_point_literal",
            "hex_integer_literal",
        ],
        tests: Tests {
            stems: &[],
            names: &[],
            directories: &["integration_test", "test_driver"],
            directory_ends: &[],
        },
        query: OnceLock::new(),
    },
    Language {
        name: "Scala",
        family: "Scala",
        extensions: &["scala"],
        grammar: || tree_sitter_scala::LANGUAGE.into(),
        tags: include_str!("queries/scala.scm"),
        blocks: &["block", "indented_block"],
        control: &[
            "if_expression",
            "match_expression",
            "for_expression",
            "while_expression",
            "do_while_expression",
            "try_expression",
        ],
        conditionals: &["if_expression"],
        clauses: &[],
        literals: &[
            "string",
            "integer_literal",
            "floating_point_literal",
            "character_literal",
        ],
        tests: Tests {
            stems: &["Test", "Tests", "Spec", "Suite", "IT"],
            names: &[],
            directories: &[],
            directory_ends: &[],
        },
        query: OnceLock::new(),
    },
    // Elixir's branches and loops are calls with `do` blocks, and its
    // functions `fn`s: those nest.
    Language {
        name: "Elixir",
        family: "Elixir",
        extensions: &["ex", "exs"],
        grammar: || tree_sitter_elixir::LANGUAGE.into(),
        tags: include_str!("queries/elixir.scm"),
        blocks: &["do_block", "body"],
        control: &["do_block", "anonymous_function"],
        conditionals: &[],
        clauses: &[],
        literals: &["quoted_content", "integer", "float", "char"],
        tests: Tests::NONE,
        query: OnceLock::new(),
    },
    Language {
        name: "Lua",
        family: "Lua",
        extensions: &["lua"],
        grammar: || tree_sitter_lua::LANGUAGE.into(),
        tags: include_str!("queries/lua.scm"),
        blocks: &["block"],
        control: &[
            "if_statement",
            "for_statement",
            "while_statement",
            "repeat_statement",
        ],
        conditionals: &["if_statement"],
        clauses: &["elseif_statement", "else_statement"],
        literals: &["string_content", "number"],
        tests: Tests {
            stems: &[],
            names: &["_spec.lua"],
            directories: &["spec"],
            directory_ends: &[],
        },
        query: OnceLock::new(),
    },
];

/// The generic language a file is written in, by its extension.
pub(crate) fn of(path: &Path) -> Option<&'static Language> {
    let extension = path.extension()?.to_str()?;
    LANGUAGES.iter().find(|language| {
        language
            .extensions
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension))
    })
}

/// The family of a file's generic language; none for the other languages,
/// which keep reading each other's code as they always have.
pub(crate) fn family(path: &Path) -> Option<&'static str> {
    of(path).map(|language| language.family)
}

/// Whether a file of a generic language is a test, by where it is and how
/// it is named.
pub(crate) fn test_path(path: &Path) -> bool {
    let Some(language) = of(path) else {
        return false;
    };
    let tests = &language.tests;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let ends = |text: &str, endings: &[&str]| {
        endings
            .iter()
            .any(|end| text.len() > end.len() && text.ends_with(end))
    };
    let directories = path.parent().into_iter().flat_map(Path::iter);
    ends(stem, tests.stems)
        || ends(&name.to_ascii_lowercase(), tests.names)
        || directories.filter_map(|part| part.to_str()).any(|part| {
            tests
                .directories
                .contains(&part.to_ascii_lowercase().as_str())
                || ends(part, tests.directory_ends)
        })
}

impl Language {
    pub(crate) fn grammar(&self) -> tree_sitter::Language {
        (self.grammar)()
    }

    fn query(&self) -> &Query {
        self.query.get_or_init(|| {
            Query::new(&self.grammar(), self.tags).expect("a generic tag query compiles")
        })
    }
}

#[cfg(test)]
mod tests;
