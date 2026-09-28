//! What marks a line as turning a check off: the suppression comments and
//! attributes of common tools, the markers that skip or focus a test, and
//! the words that make a line an assertion. Lines are matched lower-cased,
//! and a marker must start a word, so `exit(` is not `xit(`; one that ends
//! in a letter must end a word too, so `@ignore_warnings` is not `@ignore`,
//! unless it ends in `*`, which matches any ending: `@disabled*` is also
//! JUnit's `@DisabledOnOs(…)`.
use crate::analysis::regions::Region;

/// A marker on a line: the byte where it starts, what it is, and where it
/// must sit to work: a comment directive in a comment, an attribute or a
/// test marker in code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Marker<T> {
    pub at: usize,
    pub what: T,
    pub region: Region,
}

/// A marker that turns off a tool's check, the tool, for the message, and
/// the files the tool reads it in: `# noqa` in a Rust comment turns
/// nothing off.
struct Suppression {
    marker: &'static str,
    tool: &'static str,
    /// Written in a comment: only a match after a comment opener on its
    /// line counts, so a string quoting the marker is not one.
    comment: bool,
    /// File extensions the tool reads, lower-cased; empty for any file.
    files: &'static [&'static str],
}

const fn comment(
    marker: &'static str,
    tool: &'static str,
    files: &'static [&'static str],
) -> Suppression {
    Suppression {
        marker,
        tool,
        comment: true,
        files,
    }
}

const fn attribute(
    marker: &'static str,
    tool: &'static str,
    files: &'static [&'static str],
) -> Suppression {
    Suppression {
        marker,
        tool,
        comment: false,
        files,
    }
}

/// The tool a `jevgate: allow` comment turns off: JevGate itself.
pub(crate) const JEVGATE: &str = "JevGate";

/// The files of each family of tools, by extension.
const ANY: &[&str] = &[];
const PYTHON: &[&str] = &["py", "pyi"];
const SCRIPT: &[&str] = &[
    "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "vue", "svelte", "astro",
];
const STYLE: &[&str] = &[
    "css", "scss", "sass", "less", "js", "jsx", "ts", "tsx", "vue", "svelte", "astro",
];
const RUST: &[&str] = &["rs"];
const GO: &[&str] = &["go"];
const JVM: &[&str] = &["java", "kt", "kts", "scala", "groovy"];
const CSHARP: &[&str] = &["cs"];
const RUBY: &[&str] = &["rb", "rake", "erb"];
const PHP: &[&str] = &["php"];
const SHELL: &[&str] = &["sh", "bash", "zsh", "ksh", "bats"];
const SWIFT: &[&str] = &["swift"];
/// Markdown files, where only an HTML comment is a comment.
pub(super) const MARKDOWN: &[&str] = &["md", "markdown", "mdx"];

const SUPPRESSIONS: &[Suppression] = &[
    comment("jevgate: allow(", JEVGATE, ANY),
    comment("jevgate:allow(", JEVGATE, ANY),
    comment("noqa", "flake8 or Ruff", PYTHON),
    comment("type: ignore", "a type checker", PYTHON),
    comment("pyright: ignore", "Pyright", PYTHON),
    comment("pylint: disable", "Pylint", PYTHON),
    comment("pragma: no cover", "coverage", PYTHON),
    comment("mypy: ignore-errors", "mypy", PYTHON),
    comment("pyre-ignore", "Pyre", PYTHON),
    comment("pyre-fixme", "Pyre", PYTHON),
    comment("pytype: disable", "pytype", PYTHON),
    comment("eslint-disable", "ESLint", SCRIPT),
    comment("@ts-ignore", "TypeScript", SCRIPT),
    comment("@ts-expect-error", "TypeScript", SCRIPT),
    comment("@ts-nocheck", "TypeScript", SCRIPT),
    comment("tslint:disable", "TSLint", SCRIPT),
    comment("biome-ignore", "Biome", SCRIPT),
    comment("oxlint-disable", "oxlint", SCRIPT),
    comment("deno-lint-ignore", "Deno", SCRIPT),
    comment("jshint ignore", "JSHint", SCRIPT),
    comment("istanbul ignore", "coverage", SCRIPT),
    comment("c8 ignore", "coverage", SCRIPT),
    comment("v8 ignore", "coverage", SCRIPT),
    comment("stylelint-disable", "Stylelint", STYLE),
    attribute("#[allow(", "the Rust compiler or Clippy", RUST),
    attribute("#![allow(", "the Rust compiler or Clippy", RUST),
    attribute("#[expect(", "the Rust compiler or Clippy", RUST),
    attribute("#![expect(", "the Rust compiler or Clippy", RUST),
    comment("nolint", "golangci-lint", GO),
    comment("lint:ignore", "staticcheck", GO),
    attribute("@suppresswarnings", "the compiler or a linter", JVM),
    attribute("@suppress(", "the Kotlin compiler or a linter", JVM),
    attribute("@suppressfbwarnings", "SpotBugs", JVM),
    attribute("@suppresslint", "Android Lint", JVM),
    comment("checkstyle:off", "Checkstyle", JVM),
    attribute("#pragma warning disable", "the C# compiler", CSHARP),
    attribute("[suppressmessage", "a .NET analyzer", CSHARP),
    comment("resharper disable", "ReSharper", CSHARP),
    comment("rubocop:disable", "RuboCop", RUBY),
    comment("rubocop:todo", "RuboCop", RUBY),
    comment("standard:disable", "Standard", RUBY),
    comment(":nocov:", "coverage", RUBY),
    comment("phpcs:ignore*", "PHP_CodeSniffer", PHP),
    comment("phpcs:disable", "PHP_CodeSniffer", PHP),
    comment("@codingstandardsignore*", "PHP_CodeSniffer", PHP),
    comment("@phpstan-ignore", "PHPStan", PHP),
    comment("@psalm-suppress", "Psalm", PHP),
    comment("shellcheck disable", "ShellCheck", SHELL),
    comment("swiftlint:disable", "SwiftLint", SWIFT),
    comment("markdownlint-disable", "markdownlint", MARKDOWN),
    // Tools that read many languages, containers and infrastructure.
    comment("nosec", "a security scanner", ANY),
    comment("nosonar", "Sonar", ANY),
    comment("noinspection", "IntelliJ", ANY),
    comment("hadolint ignore", "Hadolint", ANY),
    comment("checkov:skip", "Checkov", ANY),
    comment("tfsec:ignore", "tfsec", ANY),
    comment("trivy:ignore", "Trivy", ANY),
    comment("yamllint disable", "yamllint", ANY),
    comment("nosemgrep", "Semgrep", ANY),
    comment("codeql[", "CodeQL", ANY),
    comment("lgtm[", "CodeQL", ANY),
    comment("gitleaks:allow", "Gitleaks", ANY),
    comment("pragma: allowlist secret", "detect-secrets", ANY),
];

/// What a test marker does to the tests it marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TestMarker {
    /// The marked test does not run, or runs without failing the suite.
    Skip,
    /// Only the marked tests run: every other test is skipped.
    Focus,
}

/// Markers that skip a test or let it fail, in the major test frameworks.
const SKIPS: &[&str] = &[
    // JavaScript and TypeScript: Jest, Vitest, Mocha, Jasmine, Playwright, Node
    "it.skip",
    "test.skip",
    "describe.skip",
    "context.skip",
    "suite.skip",
    "specify.skip",
    "bench.skip",
    "it.todo(",
    "test.todo(",
    "test.fixme(",
    "this.skip(",
    "xit(",
    "xtest(",
    "xdescribe(",
    "xcontext(",
    "xspecify(",
    // Python: pytest and unittest
    "@pytest.mark.skip",
    "@pytest.mark.skipif",
    "@pytest.mark.xfail",
    "pytest.skip(",
    "pytest.xfail(",
    "pytest.importorskip(",
    "@unittest.skip",
    "@unittest.skipif",
    "@unittest.skipunless",
    "@unittest.expectedfailure",
    "@skip(",
    "@skipif(",
    "@skipunless(",
    "skiptest(",
    "raise skiptest",
    "raise unittest.skiptest",
    // Rust and Go
    "#[ignore",
    "t.skip(",
    "t.skipf(",
    "t.skipnow(",
    "b.skip(",
    "b.skipnow(",
    // Java and Kotlin: JUnit
    "@disabled*",
    "@ignore",
    // C#: xUnit, NUnit, MSTest
    "[ignore",
    "[fact(skip",
    "[theory(skip",
    "assert.ignore(",
    "assert.inconclusive(",
    // PHP: PHPUnit and Pest
    "marktestskipped(",
    "marktestincomplete(",
    "->skip(",
    "->todo(",
    // Swift: XCTest
    "xctskip(",
];

/// Markers after which only the marked tests run.
const FOCUSES: &[&str] = &[
    "it.only",
    "test.only",
    "describe.only",
    "context.only",
    "suite.only",
    "specify.only",
    "fit(",
    "fdescribe(",
    "fcontext(",
    "fspecify(",
    "focus: true",
];

/// Ruby statements that skip the example they open or sit in, when they
/// start the line: RSpec `skip`, `pending`, `xit` and Minitest `skip`.
const RUBY_SKIPS: &[&str] = &[
    "skip",
    "pending",
    "xit",
    "xspecify",
    "xdescribe",
    "xcontext",
];

/// Words that make a test line an assertion, in the major test frameworks:
/// `assert…`, `expect(…)`, `should`, `verify(…)`, `refute…`, testify's
/// `require.…`, Go's `t.Error…` and `t.Fatal…`, minitest's `must_…`, and
/// ava's and tap's `t.is(…)`, `t.deepEqual(…)`, `t.true(…)` and the like.
const ASSERTIONS: &[&str] = &[
    "assert",
    "expect",
    "should",
    "verify",
    "refute",
    "require.",
    "t.error",
    "t.fatal",
    "t.fail",
    "must_",
    "wont_",
    "t.is(",
    "t.deepequal",
    "t.true(",
    "t.false(",
    "t.truthy(",
    "t.falsy(",
    "t.throws",
    "t.like(",
    "t.regex(",
    "t.not",
];

/// The suppressions `line`, of a file with `extension` (lower-cased), may
/// hold, in the order of the table and then of the line, with the tool
/// whose check each turns off. A comment directive counts only after a
/// comment opener (in Markdown, `<!--`); whether each sits where it works
/// is for the caller to tell.
pub(super) fn suppressions(line: &str, extension: &str) -> Vec<Marker<&'static str>> {
    let lower = line.to_ascii_lowercase();
    let markdown = MARKDOWN.contains(&extension);
    SUPPRESSIONS
        .iter()
        .filter(|s| s.files.is_empty() || s.files.contains(&extension))
        .flat_map(|s| {
            let region = if s.comment {
                Region::Comment
            } else {
                Region::Code
            };
            find_all(&lower, s.marker)
                .filter(|&at| !s.comment || commented(&lower[..at], markdown))
                .filter(|_| !markdown || s.comment)
                .map(move |at| Marker {
                    at,
                    what: s.tool,
                    region,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The test markers `line` may hold and what each does, focus first;
/// `ruby` adds the statements that skip an RSpec or Minitest example. Each
/// must sit in code, which is for the caller to tell.
pub(super) fn test_markers(line: &str, ruby: bool) -> Vec<Marker<TestMarker>> {
    let lower = line.to_ascii_lowercase();
    let code = |what| {
        move |at| Marker {
            at,
            what,
            region: Region::Code,
        }
    };
    let mut found: Vec<Marker<TestMarker>> = FOCUSES
        .iter()
        .flat_map(|m| find_all(&lower, m).map(code(TestMarker::Focus)))
        .collect();
    found.extend(
        SKIPS
            .iter()
            .flat_map(|m| find_all(&lower, m).map(code(TestMarker::Skip))),
    );
    if ruby && ruby_skip(lower.trim_start()) {
        found.push(code(TestMarker::Skip)(
            lower.len() - lower.trim_start().len(),
        ));
    }
    found
}

/// Whether `line` asserts something about the code under test.
pub(super) fn assertion(line: &str) -> bool {
    let lower = line.to_lowercase();
    ASSERTIONS.iter().any(|word| lower.contains(word))
}

/// The assertion lines of a test's `source`, trimmed: each line naming an
/// assertion, and the `if` line that guards one, where Go writes what it
/// checks (`if got != 3 {` above `t.Fatalf(…)`).
pub(super) fn assertion_lines(source: &str) -> Vec<&str> {
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let mut found = Vec::new();
    for (at, &line) in lines.iter().enumerate() {
        if !assertion(line) {
            continue;
        }
        let guard = lines[..at]
            .iter()
            .rev()
            .find(|l| !l.is_empty())
            .filter(|l| l.starts_with("if ") && l.ends_with('{') && !assertion(l));
        found.extend(guard.copied());
        found.push(line);
    }
    found
}

/// A Ruby skip statement opening `line`: the word alone or followed by an
/// argument, as in `skip "flaky"` or `pending("not built")`.
fn ruby_skip(line: &str) -> bool {
    RUBY_SKIPS.iter().any(|word| {
        line.strip_prefix(word)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '(', '"', '\'']))
    })
}

/// Where `marker` occurs in `line` as words: after a character that is not
/// a letter, digit or underscore, and before one unless it ends in `*`.
/// Punctuation at either end (`#[allow(`, `@ts-ignore`) needs no such
/// neighbour.
fn find_all<'a>(line: &'a str, marker: &'a str) -> impl Iterator<Item = usize> + 'a {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let (marker, any_ending) = match marker.strip_suffix('*') {
        Some(prefix) => (prefix, true),
        None => (marker, false),
    };
    let (opens, closes) = (marker.starts_with(word), marker.ends_with(word));
    line.match_indices(marker)
        .map(|(at, _)| at)
        .filter(move |&at| {
            let after = &line[at + marker.len()..];
            (!opens || !line[..at].ends_with(word))
                && (any_ending || !closes || !after.starts_with(word))
        })
}

/// Whether `before`, the text before a marker on its line, opens a comment;
/// in `markdown`, an HTML one.
fn commented(before: &str, markdown: bool) -> bool {
    if markdown {
        return before.contains("<!--");
    }
    let trimmed = before.trim_start();
    ["#", "//", "/*", "--", "<!--"]
        .iter()
        .any(|opener| before.contains(opener))
        || trimmed.starts_with('*')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the first test marker on `line` does, wherever it sits.
    fn skip(line: &str, ruby: bool) -> Option<TestMarker> {
        test_markers(line, ruby).first().map(|m| m.what)
    }

    /// The tool of the first suppression on `line`, of a file with
    /// `extension`, wherever it sits.
    fn suppression(line: &str, extension: &str) -> Option<&'static str> {
        suppressions(line, extension).first().map(|m| m.what)
    }

    /// Where `marker` first occurs in `line` as words.
    fn find(line: &str, marker: &str) -> Option<usize> {
        find_all(line, marker).next()
    }

    #[test]
    fn suppressions_of_each_family_name_their_tool() {
        for (line, extension, tool) in [
            ("    x = compute()  # noqa: E501", "py", "flake8 or Ruff"),
            (
                "value = cast(Any, x)  # type: ignore[arg-type]",
                "py",
                "a type checker",
            ),
            ("// eslint-disable-next-line no-console", "tsx", "ESLint"),
            ("  // @ts-expect-error the typings lag", "ts", "TypeScript"),
            ("#[allow(dead_code)]", "rs", "the Rust compiler or Clippy"),
            (
                "#![expect(clippy::too_many_lines, reason = \"x\")]",
                "rs",
                "the Rust compiler or Clippy",
            ),
            ("\tresult := run() //nolint:errcheck", "go", "golangci-lint"),
            (
                "    @SuppressWarnings(\"unchecked\")",
                "java",
                "the compiler or a linter",
            ),
            ("#pragma warning disable CS0618", "cs", "the C# compiler"),
            (
                "  def call # rubocop:disable Metrics/AbcSize",
                "rb",
                "RuboCop",
            ),
            ("     * @phpstan-ignore-next-line", "php", "PHPStan"),
            ("# shellcheck disable=SC2086", "sh", "ShellCheck"),
            ("<!-- markdownlint-disable MD013 -->", "md", "markdownlint"),
            (
                "key = 'x'  # nosemgrep: python.lang.security",
                "py",
                "Semgrep",
            ),
            (
                "fn f() {} // jevgate: allow(shared_logic) mirrors g",
                "rs",
                JEVGATE,
            ),
        ] {
            assert_eq!(suppression(line, extension), Some(tool), "{line}");
        }
    }

    #[test]
    fn a_marker_outside_a_comment_inside_a_word_or_of_another_language_is_not_a_suppression() {
        for (line, extension) in [
            ("const RULE = \"eslint-disable\";", "ts"),
            ("rules.push(\"noqa\")", "py"),
            ("let nolinter = 3;", "go"),
            ("fn allow(x: i32) {}", "rs"),
            ("annotate(\"@ts-ignore\")", "ts"),
            ("// Python ignores this with `# noqa`.", "rs"),
            ("x = 1  # eslint-disable-line", "py"),
            ("Add `# noqa` after the import.", "md"),
            ("- `#[allow(dead_code)]` keeps it quiet", "md"),
        ] {
            assert_eq!(suppression(line, extension), None, "{line}");
        }
    }

    #[test]
    fn skip_and_focus_markers_start_a_word() {
        for line in [
            "it.skip('adds', () => {",
            "  xit(\"totals\", function () {",
            "@pytest.mark.skip(reason=\"flaky\")",
            "        self.skipTest(\"needs network\")",
            "#[ignore = \"slow\"]",
            "\tt.Skip(\"not on windows\")",
            "  @Disabled(\"broken\")",
            "    [Fact(Skip = \"flaky\")]",
            "        $this->markTestSkipped('no driver');",
        ] {
            assert_eq!(skip(line, false), Some(TestMarker::Skip), "{line}");
        }
        for line in [
            "    skip \"pending a fix\"",
            "    pending",
            "  xit \"totals\" do",
        ] {
            assert_eq!(skip(line, true), Some(TestMarker::Skip), "{line}");
            assert_eq!(skip(line, false), None, "only Ruby: {line}");
        }
        for line in ["describe.only('cart', () => {", "  fit('totals', () => {"] {
            assert_eq!(skip(line, false), Some(TestMarker::Focus), "{line}");
        }
        for line in [
            "process.exit(1)",
            "const profit = total - cost;",
            "let skipped = items.skip(2);",
            "items.Skip(3).ToList();",
            "hint.skip(1)",
            "skipper = 2",
            "@ignore_warnings(category=ConvergenceWarning)",
            "flags = [ignore_case, multiline]",
            "it.skipping_rows()",
        ] {
            assert_eq!(skip(line, true), None, "{line}");
        }
        for line in [
            "@pytest.mark.skipif(sys.platform == \"win32\", reason=\"paths\")",
            "  @DisabledOnOs(OS.WINDOWS)",
            "@Ignore",
            "test.skip.each([[1, 2]])('adds %i', (a) => {",
        ] {
            assert_eq!(skip(line, false), Some(TestMarker::Skip), "{line}");
        }
    }

    #[test]
    fn a_marker_ends_a_word_unless_it_ends_in_a_star() {
        assert_eq!(find("@ignore(\"slow\")", "@ignore"), Some(0));
        assert_eq!(find("@ignoreall", "@ignore"), None);
        assert_eq!(find("@disabledif(\"x\")", "@disabled*"), Some(0));
        assert_eq!(find("#[ignore]", "#[ignore"), Some(0));
        assert_eq!(
            suppression("// phpcs:ignoreFile", "php"),
            Some("PHP_CodeSniffer"),
            "a prefix"
        );
    }

    #[test]
    fn assertions_are_named_by_their_framework_words() {
        for line in [
            "assert_eq!(total, 3);",
            "expect(total).toBe(3)",
            "self.assertEqual(total, 3)",
            "total.should eq(3)",
            "require.NoError(t, err)",
            "\tt.Errorf(\"got %d\", total)",
        ] {
            assert!(assertion(line), "{line}");
        }
        assert!(!assertion("let total = add(1, 2);"));
        assert_eq!(
            assertion_lines(
                "got := add(1, 2)\nif got != 3 {\n\tt.Fatalf(\"got %d\", got)\n}\nt.is(total, 3)\n"
            ),
            [
                "if got != 3 {",
                "t.Fatalf(\"got %d\", got)",
                "t.is(total, 3)"
            ]
        );
    }
}
